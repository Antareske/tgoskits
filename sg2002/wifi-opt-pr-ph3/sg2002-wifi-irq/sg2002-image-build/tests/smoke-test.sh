#!/usr/bin/env bash
set -euo pipefail
SKILL="$(cd "$(dirname "$0")/.." && pwd)"
T=/tmp/sg2002-skill-test
rm -rf "$T"; mkdir -p "$T/assets" "$T/host"
cd "$T"
PASS=0; FAIL=0
ok() { PASS=$((PASS+1)); echo "  [ok] $1"; }
bad() { FAIL=$((FAIL+1)); echo "  [FAIL] $1"; }

dd if=/dev/urandom of=host/fip.bin bs=440832 count=1 status=none
dd if=/dev/urandom of=host/ramdisk.bin bs=2527648 count=1 status=none
dd if=/dev/urandom of=host/starryos.bin bs=1048576 count=1 status=none
dd if=/dev/urandom of=host/dtb.bin bs=24704 count=1 status=none
echo 'fake sh' > host/fake-sh
echo 'fake app' > host/fake-app
echo 'echo hello' > host/profile-v1

# 1. inject
truncate -s 32M rootfs.ext4 && mkfs.ext4 -q rootfs.ext4
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 \
  --inject host/fake-app:/root/deep/act-infer:0755 \
  --inject host/fake-sh:/bin/sh >/dev/null
debugfs -R "stat /root/deep/act-infer" rootfs.ext4 2>/dev/null | grep -q "^Inode:" && ok "inject: 自动建父目录+写文件" || bad "inject: 自动建父目录"
debugfs -R "stat /root/deep/act-infer" rootfs.ext4 2>/dev/null | grep -qE "Type: *regular" && debugfs -R "stat /root/deep/act-infer" rootfs.ext4 2>/dev/null | grep -qE "Mode:.*0755" && ok "inject: mode 0755 (Type regular)" || bad "inject: mode"
debugfs -R "stat /bin/sh" rootfs.ext4 2>/dev/null | grep -q "^Inode:" && ok "inject: /bin/sh" || bad "inject: /bin/sh"
echo 'fake app v2' > host/fake-app-v2
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 --inject host/fake-app-v2:/root/deep/act-infer:0755 >/dev/null
debugfs -R "cat /root/deep/act-infer" rootfs.ext4 2>/dev/null | grep -q "v2" && ok "inject: 覆盖已存在文件" || bad "inject: 覆盖"

# 2. init-assets 幂等
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 --inject host/profile-v1:/etc/profile >/dev/null
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 --init-assets >/dev/null
debugfs -R "cat /etc/profile" rootfs.ext4 2>/dev/null > /tmp/profile.out
grep -q "starry-init" /tmp/profile.out && ok "init-assets: profile 追加" || bad "init-assets: profile 追加"
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 --init-assets >/dev/null
debugfs -R "cat /etc/profile" rootfs.ext4 2>/dev/null > /tmp/profile2.out
[ "$(grep -c starry-init /tmp/profile2.out)" = "1" ] && ok "init-assets: 幂等" || bad "init-assets: 幂等"
debugfs -R "stat /usr/bin/starry-init.sh" rootfs.ext4 2>/dev/null | grep -qE "Mode:.*0755" && ok "init-assets: starry-init.sh 0755" || bad "init-assets: starry-init.sh"

# 3. repack-fit
cp host/starryos.bin host/ramdisk.bin host/dtb.bin assets/
cp host/dtb.bin assets/licheerv-nano-sg2002.dtb
( cd assets && ASSETS_DIR="$T/assets" ITS="$SKILL/templates/boot.its" "$SKILL/scripts/repack-fit.sh" >/dev/null )
mkimage -l assets/boot.sd | grep -q "Default Configuration:.*config-sg2002_licheervnano_sd" && ok "repack: 默认配置名" || bad "repack: 默认配置名"
mkimage -l assets/boot.sd | grep -q "Load Address:.*80200000" && ok "repack: Load=0x80200000" || bad "repack: Load 地址"

# 4. build-image (SIZE_MB=100)
cp rootfs.ext4 assets/
( cd assets && ASSETS_DIR="$T/assets" FIP="$T/host/fip.bin" "$SKILL/scripts/build-image.sh" "$T/out.img" >/dev/null )
sfdisk -l out.img 2>/dev/null | grep -qE "[cC]  *W95" && ok "build: FAT 分区 type=c" || bad "build: FAT 分区"
mdir -i "out.img@@$((2048*512))" :: | grep -q "fip" && ok "build: FAT 含 fip.bin" || bad "build: FAT fip.bin"
mdir -i "out.img@@$((2048*512))" :: | grep -q "boot" && ok "build: FAT 含 boot.sd" || bad "build: FAT boot.sd"
debugfs -R "stat /bin/sh" "out.img?offset=$((133120*512))" 2>/dev/null | grep -q "^Inode:" && ok "build: p2 含 /bin/sh" || bad "build: p2 /bin/sh"

# 5. swap-kernel 就地 + p2 完整性 (回归: mcopy 直写整盘镜像曾清零 p2)
dd if=/dev/urandom of=host/starryos-v2.bin bs=1048576 count=1 status=none
ASSETS_DIR="$T/assets" "$SKILL/scripts/swap-kernel.sh" out.img host/starryos-v2.bin >/dev/null
mdir -i "out.img@@$((2048*512))" :: | grep -q "boot" && ok "swap-kernel: 写回 boot.sd" || bad "swap-kernel"
debugfs -R "stat /bin/sh" "out.img?offset=$((133120*512))" 2>/dev/null | grep -q "^Inode:" && ok "swap-kernel: p2 保持完整" || bad "swap-kernel: p2 被破坏"

# 6. swap-rootfs 原地 + 扩分区
"$SKILL/scripts/inject-rootfs.sh" rootfs.ext4 --inject host/fake-app-v2:/root/marker >/dev/null
ASSETS_DIR="$T/assets" "$SKILL/scripts/swap-rootfs.sh" out.img rootfs.ext4 >/dev/null
debugfs -R "stat /root/marker" "out.img?offset=$((133120*512))" 2>/dev/null | grep -q "^Inode:" && ok "swap-rootfs: 原地写入" || bad "swap-rootfs: 原地"
truncate -s 40M rootfs-big.ext4 && mkfs.ext4 -q rootfs-big.ext4
"$SKILL/scripts/inject-rootfs.sh" rootfs-big.ext4 --inject host/fake-sh:/bin/sh --inject host/fake-app:/root/big-marker >/dev/null
ASSETS_DIR="$T/assets" "$SKILL/scripts/swap-rootfs.sh" out.img rootfs-big.ext4 >/dev/null
P2=$(sfdisk -d out.img | grep 'type=83' | grep -oP 'start=\s*\K\d+')
debugfs -R "stat /root/big-marker" "out.img?offset=$((P2*512))" 2>/dev/null | grep -q "^Inode:" && ok "swap-rootfs: 扩分区后写入" || bad "swap-rootfs: 扩分区"
[ "$(stat -c%s out.img)" -gt 100000000 ] && ok "swap-rootfs: 镜像已扩大" || bad "swap-rootfs: 扩大"

# 7. provision 偏移自动探测
dd if=/dev/zero of=official.img bs=512 count=0 seek=16384 status=none
mkfs.fat -F 12 --offset 2048 official.img >/dev/null 2>&1
cat > host/mini.its <<'EOF'
/dts-v1/;
/ {
  description = "fake";
  #address-cells = <1>;
  images {
    kernel-1 { description = "k"; data = /incbin/("starryos.bin"); type = "kernel"; arch = "riscv"; compression = "none"; hash-1 { algo = "crc32"; }; };
    ramdisk-1 { description = "r"; data = /incbin/("ramdisk.bin"); type = "ramdisk"; arch = "riscv"; compression = "none"; hash-1 { algo = "crc32"; }; };
  };
  configurations { default = "c"; c { kernel = "kernel-1"; ramdisk = "ramdisk-1"; }; };
};
EOF
( cd host && mkimage -f mini.its boot.sd >/dev/null )
OFF=$((2048*512))
mcopy -i "official.img@@$OFF" host/fip.bin ::fip.bin
mcopy -i "official.img@@$OFF" host/boot.sd ::boot.sd
"$SKILL/scripts/provision-sg2002.sh" official.img "$T/prov-assets" >/dev/null
cmp -s host/fip.bin prov-assets/fip.bin && ok "provision: fip.bin 提取一致" || bad "provision: fip.bin"
cmp -s host/ramdisk.bin prov-assets/ramdisk.bin && ok "provision: ramdisk.bin 提取一致" || bad "provision: ramdisk.bin"

# 8. 编排入口
mkdir -p "$T/work/assets" "$T/work/output" "$T/work/work"
cp rootfs.ext4 "$T/work/assets/rootfs.ext4"
"$SKILL/scripts/sg2002-image-build.sh" inject --work "$T/work" --inject host/fake-app:/root/orch-marker >/dev/null
debugfs -R "stat /root/orch-marker" "$T/work/assets/rootfs.ext4" 2>/dev/null | grep -q "^Inode:" && ok "orchestrator: inject 子命令" || bad "orchestrator: inject"
"$SKILL/scripts/sg2002-image-build.sh" check-deps >/dev/null && ok "orchestrator: check-deps" || bad "orchestrator: check-deps"
"$SKILL/scripts/sg2002-image-build.sh" >/dev/null 2>&1 && ok "orchestrator: 无参数显示用法" || bad "orchestrator: 用法"

# 8.5 update-kernel 端到端: 新镜像 + uimg 同步 + p2 完好 + buildinfo
mkdir -p "$T/work2/assets" "$T/work2/output" "$T/work2/work"
cp host/fip.bin host/ramdisk.bin host/dtb.bin "$T/work2/assets/"
cp host/dtb.bin "$T/work2/assets/licheerv-nano-sg2002.dtb"
cp host/starryos.bin "$T/work2/assets/starryos.bin"
mkimage -q -A riscv -O linux -T kernel -C none -a 0x80200000 -e 0x80200000 -d host/starryos-v2.bin host/starryos-v2.uimg
"$SKILL/scripts/sg2002-image-build.sh" update-kernel out.img --work "$T/work2" \
  --kernel host/starryos-v2.bin -o "$T/out-kernel.img" >/dev/null
mdir -i "out-kernel.img@@$((2048*512))" :: | grep -q "boot" && ok "update-kernel: 新镜像 boot.sd" || bad "update-kernel: boot.sd"
debugfs -R "stat /bin/sh" "out-kernel.img?offset=$((133120*512))" 2>/dev/null | grep -q "^Inode:" && ok "update-kernel: p2 完好" || bad "update-kernel: p2 被破坏"
debugfs -R "stat /starryos.uimg" "out-kernel.img?offset=$((133120*512))" 2>/dev/null | grep -q "^Inode:" && ok "update-kernel: uimg 同步到 rootfs" || bad "update-kernel: uimg 同步"
cmp -s host/starryos-v2.uimg "$T/work2/assets/starryos.uimg" && ok "update-kernel: uimg 资产配对" || bad "update-kernel: uimg 资产配对"
grep -q '"operation": "update-kernel"' out-kernel.img.json && ok "update-kernel: buildinfo" || bad "update-kernel: buildinfo"
[ "$(readlink "$T/work2/output/latest.img")" = "$(realpath out-kernel.img)" ] && ok "update-kernel: latest.img 更新" || bad "update-kernel: latest.img"

echo
echo "== 结果: $PASS 通过, $FAIL 失败 =="
[ "$FAIL" -eq 0 ]
