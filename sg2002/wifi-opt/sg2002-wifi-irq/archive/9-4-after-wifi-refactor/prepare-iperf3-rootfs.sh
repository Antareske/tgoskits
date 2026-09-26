#!/usr/bin/env bash
#
# prepare-iperf3-rootfs.sh — Install iperf3 into Alpine riscv64 rootfs image
#
# Extract rootfs with fakeroot, install iperf3 via qemu-user,
# collect changed files into an overlay, inject with debugfs.
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ORIGINAL_ROOTFS="$REPO_ROOT/tmp/axbuild/rootfs/rootfs-riscv64-alpine.img/rootfs-riscv64-alpine.img"
WORK_ROOTFS="$REPO_ROOT/target/sg2002/rootfs-riscv64-alpine-iperf3.img"
STAGING_DIR="$REPO_ROOT/target/sg2002/rootfs-staging"
OVERLAY_DIR="$REPO_ROOT/target/sg2002/rootfs-overlay"
APK_CACHE="$REPO_ROOT/target/sg2002/apk-cache"
QEMU_RUNNER="qemu-riscv64-static"

echo "=== Step 1: Copy original rootfs ==="
cp "$ORIGINAL_ROOTFS" "$WORK_ROOTFS"

echo "=== Step 2: Extract rootfs to staging ==="
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"
fakeroot -- debugfs -R "rdump / $STAGING_DIR" "$WORK_ROOTFS" 2>/dev/null

if [[ ! -e "$STAGING_DIR/bin/sh" ]] && [[ ! -e "$STAGING_DIR/bin/busybox" ]]; then
    echo "ERROR: rootfs extraction failed!"
    exit 1
fi
echo "Rootfs extracted: $(find "$STAGING_DIR" -type f | wc -l) files"

echo "=== Step 3: Install iperf3 via apk (qemu-user) ==="
mkdir -p "$APK_CACHE"
cp /etc/resolv.conf "$STAGING_DIR/etc/resolv.conf"

# Save snapshot of files before apk
find "$STAGING_DIR" -not -type d | sort > "$REPO_ROOT/target/sg2002/files-before.txt"

QEMU_LD_PREFIX="$STAGING_DIR" LD_LIBRARY_PATH="$STAGING_DIR/lib:$STAGING_DIR/usr/lib" \
    "$QEMU_RUNNER" -L "$STAGING_DIR" "$STAGING_DIR/sbin/apk" \
        --root "$STAGING_DIR" \
        --repositories-file "$STAGING_DIR/etc/apk/repositories" \
        --keys-dir "$STAGING_DIR/etc/apk/keys" \
        --cache-dir "$APK_CACHE" \
        --update-cache --timeout 60 --no-interactive --force-no-chroot --scripts=no \
        add iperf3

if [[ ! -f "$STAGING_DIR/usr/bin/iperf3" ]]; then
    echo "ERROR: iperf3 was not installed!"
    exit 1
fi
echo "iperf3 installed: $(stat -c%s "$STAGING_DIR/usr/bin/iperf3") bytes"

echo "=== Step 4: Collect changed files into overlay ==="
find "$STAGING_DIR" -not -type d | sort > "$REPO_ROOT/target/sg2002/files-after.txt"
comm -13 "$REPO_ROOT/target/sg2002/files-before.txt" "$REPO_ROOT/target/sg2002/files-after.txt" > "$REPO_ROOT/target/sg2002/files-new.txt"
NEW_COUNT=$(wc -l < "$REPO_ROOT/target/sg2002/files-new.txt")
echo "New/changed files: $NEW_COUNT"

rm -rf "$OVERLAY_DIR"
while IFS= read -r file; do
    rel="${file#$STAGING_DIR/}"
    dest="$OVERLAY_DIR/$rel"
    mkdir -p "$(dirname "$dest")"
    if [[ -L "$file" ]]; then
        target=$(readlink "$file")
        # Convert relative symlink target to absolute for debugfs
        if [[ "$target" != /* ]]; then
            guest_dir="/$(dirname "$rel")"
            target="$guest_dir/$target"
        fi
        # Store symlink info: dest contains the target path
        echo -n "$target" > "$dest"
        # Mark as symlink by setting a special permission (we detect it by the file being in the new-files list and being a symlink in staging)
    else
        cp -a "$file" "$dest"
    fi
done < "$REPO_ROOT/target/sg2002/files-new.txt"

echo "Overlay created: $(find "$OVERLAY_DIR" -type f -o -type l | wc -l) entries"

echo "=== Step 5: Generate debugfs script ==="
DEBUGFS_SCRIPT="$REPO_ROOT/target/sg2002/debugfs-inject.txt"
> "$DEBUGFS_SCRIPT"

# Collect all entries from overlay (files/symlinks), sort by depth so parent dirs are created first
find "$OVERLAY_DIR" -not -type d | sort > "$REPO_ROOT/target/sg2002/overlay-files.txt"

# Phase 1: Create directories (collect unique dirs from overlay entries)
while IFS= read -r file; do
    rel="${file#$OVERLAY_DIR/}"
    dir="/$(dirname "$rel")"
    echo "$dir"
done < "$REPO_ROOT/target/sg2002/overlay-files.txt" | sort -u | while IFS= read -r dir; do
    echo "mkdir $dir"
done >> "$DEBUGFS_SCRIPT"

# Phase 2: Write regular files (not symlinks)
while IFS= read -r file; do
    rel="${file#$OVERLAY_DIR/}"
    src_in_staging="$STAGING_DIR/$rel"
    if [[ ! -L "$src_in_staging" ]]; then
        echo "rm /$rel"
        echo "write $file /$rel"
    fi
done < "$REPO_ROOT/target/sg2002/overlay-files.txt" >> "$DEBUGFS_SCRIPT"

# Phase 3: Write symlinks (after targets are in place)
while IFS= read -r file; do
    rel="${file#$OVERLAY_DIR/}"
    src_in_staging="$STAGING_DIR/$rel"
    if [[ -L "$src_in_staging" ]]; then
        # The overlay file contains the symlink target (made absolute above)
        target=$(cat "$file")
        echo "rm /$rel"
        echo "symlink /$rel $target"
    fi
done < "$REPO_ROOT/target/sg2002/overlay-files.txt" >> "$DEBUGFS_SCRIPT"

echo "quit" >> "$DEBUGFS_SCRIPT"

echo "Debugfs script: $(wc -l < "$DEBUGFS_SCRIPT") commands"

echo "=== Step 6: Inject overlay into rootfs image ==="
debugfs -w "$WORK_ROOTFS" < "$DEBUGFS_SCRIPT" 2>&1 | grep -v "already exists\|File exists" | tail -5

echo ""
echo "=== Step 7: Cleanup ==="
rm -rf "$STAGING_DIR" "$OVERLAY_DIR" "$APK_CACHE" \
    "$REPO_ROOT/target/sg2002/files-before.txt" \
    "$REPO_ROOT/target/sg2002/files-after.txt" \
    "$REPO_ROOT/target/sg2002/files-new.txt" \
    "$REPO_ROOT/target/sg2002/overlay-files.txt" \
    "$DEBUGFS_SCRIPT"

echo "=== Done ==="
echo "Rootfs with iperf3: $WORK_ROOTFS"
echo "Size: $(stat -c%s "$WORK_ROOTFS") bytes"

# Quick verification
echo "Verifying /usr/bin/iperf3 ..."
debugfs -R "stat /usr/bin/iperf3" "$WORK_ROOTFS" 2>&1 | head -5
