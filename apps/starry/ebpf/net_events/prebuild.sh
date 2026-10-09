#!/usr/bin/env bash
# Build net-events (aya loader + embedded eBPF bytecode) as a static musl binary
# and install it into the StarryOS rootfs overlay.
#
# The eBPF programs are compiled by aya-build with the `nightly` toolchain, and
# the resulting LLVM bitcode must be readable by the installed `bpf-linker`.
set -euo pipefail

app_dir="${STARRY_APP_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
overlay_dir="${STARRY_OVERLAY_DIR:?STARRY_OVERLAY_DIR is required}"
arch="${STARRY_ARCH:-x86_64}"

case "$arch" in
    x86_64)  musl_target="x86_64-unknown-linux-musl";    cross_prefix="x86_64-linux-musl" ;;
    aarch64) musl_target="aarch64-unknown-linux-musl";   cross_prefix="aarch64-linux-musl" ;;
    riscv64) musl_target="riscv64gc-unknown-linux-musl"; cross_prefix="riscv64-linux-musl" ;;
    *) echo "net-events prebuild: unsupported arch '$arch'" >&2; exit 1 ;;
esac

cross_bin="/opt/${cross_prefix}-cross/bin"
if [[ -d "$cross_bin" ]]; then
    export PATH="$cross_bin:$PATH"
fi
cc_bin="${cross_prefix}-gcc"

if ! command -v rustup >/dev/null 2>&1; then
    echo "net-events prebuild: rustup is required to install Rust target $musl_target" >&2
    exit 1
fi
read -r rust_toolchain _ < <(rustup show active-toolchain)
echo "net-events prebuild: installing Rust target $musl_target for toolchain $rust_toolchain"
rustup target add --toolchain "$rust_toolchain" "$musl_target"
export RUSTUP_TOOLCHAIN="$rust_toolchain"

host_tools_dir="${STARRY_WORKSPACE:-$app_dir}/tmp/axbuild/starry-host-tools"
export PATH="$host_tools_dir/bin:${HOME:-/root}/.cargo/bin:$PATH"
if ! command -v bpf-linker >/dev/null 2>&1; then
    if command -v apk >/dev/null 2>&1; then
        echo "net-events prebuild: installing bpf-linker with apk"
        apk add --no-cache bpf-linker || true
    fi
    if ! command -v bpf-linker >/dev/null 2>&1; then
        echo "net-events prebuild: installing bpf-linker 0.11.1 with cargo"
        cargo install bpf-linker --version 0.11.1 --locked --root "$host_tools_dir"
    fi
fi

echo "net-events prebuild: building for $musl_target (CC=$cc_bin)"
(
    cd "$app_dir"
    # Static linking has to be asked for: Rust does not enable `crt-static` for
    # this target, and the dynamic build carries NEEDED entries for the musl
    # loader and libgcc. The binary ships into root filesystems the build does
    # not control, so it must not depend on what they happen to provide.
    CC="$cc_bin" cargo build --release --target "$musl_target" \
        --config "target.${musl_target}.linker=\"${cc_bin}\"" \
        --config "target.${musl_target}.rustflags=[\"-C\",\"target-feature=+crt-static\"]"
)

bin="$app_dir/target/$musl_target/release/net-events"
[[ -x "$bin" ]] || { echo "net-events prebuild: build did not produce $bin" >&2; exit 1; }

install -Dm0755 "$bin" "$overlay_dir/usr/bin/net-events"

# The case's check runs from inside the guest, in one short line; see
# net-events-check.sh for why it is a script rather than a longer shell_cmd.
install -Dm0755 "$app_dir/net-events-check.sh" "$overlay_dir/usr/bin/net-events-check"

# The QEMU root filesystem has no iperf3 while the boards ship one. Pointing
# this at a directory that holds an iperf3 for this target (and libiperf, when
# the build is dynamic) puts them in the overlay, so the QEMU run can be driven
# with the same command the board is driven with.
if [[ -n "${NET_EVENTS_IPERF3_DIR:-}" ]]; then
    install -Dm0755 "$NET_EVENTS_IPERF3_DIR/iperf3" "$overlay_dir/usr/bin/iperf3"
    if [[ -f "$NET_EVENTS_IPERF3_DIR/libiperf.so.0" ]]; then
        install -Dm0644 "$NET_EVENTS_IPERF3_DIR/libiperf.so.0" \
            "$overlay_dir/usr/lib/libiperf.so.0"
    fi
    echo "net-events prebuild: iperf3 installed from $NET_EVENTS_IPERF3_DIR"
fi

echo "net-events prebuild: installed -> /usr/bin/net-events"
