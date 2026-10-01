# dev 主线 wifi 镜像构建小结（2026-09-03）

- 产物：`/workspace/tgoskits/.sg2002-build/output/sg2002_starryos_20260903-083455.img`（约 2.1 GiB，`latest.img` 已更新）
- 构建来源：`/workspace/tgoskits` 工作树当前状态（dev HEAD `15dee37d2`，未检出远端提交）
- 板级配置：`os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml`（features 含 `ax-driver/aic8800-wifi`，aic8800 固件 `include_bytes!` 编入内核，无需注入）
- DTB：回退主线 `os/StarryOS/configs/board/licheerv-nano-sg2002.dtb`（22679 字节）
- rootfs：复用 `assets/rootfs.ext4`（Alpine riscv64），注入 `www/sg2002-iperf3-inject/usr/` 下的 `/usr/bin/iperf3`（9952 B）与 `/usr/lib/libiperf.so.0`（168016 B，实文件非 symlink），均在最终镜像 p2 用 debugfs 复核通过
- `/starryos.uimg` 已注入 rootfs，支持 U-Boot 手动引导回退
- boot.sd 校验：Load=0x80200000、默认配置 `config-sg2002_licheervnano_sd`，`/bin/sh` 存在
- 构建前把 provision 资产 `fip.bin`（440832 B）/`ramdisk.bin`（2527648 B）从 `wt-sg2002-wifi-irq/.sg2002-build/assets` 复制到本树 assets（此前缺失）
- aic8800 离线编译：`AIC8800_FIRMWARE_DIR=<target>/build/aic8800-aeaac3537c0ec0a1/out/firmware`（12 blob）
