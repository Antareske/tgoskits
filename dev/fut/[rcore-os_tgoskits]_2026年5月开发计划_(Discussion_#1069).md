# 开发计划

1. 阶段 **2026.5.1** ~ **2026.5.31**

2. 任务根据功能分类并以无序列表的形式展现，并在编写时采用 Markdown 的 `- [ ]` 开头以显示复选框，选中复选框表示任务已经完成

3. 任务格式：`【进度百分比】[可选✨符号] 任务名字或者直接写对应的 PR @负责人`，其中的 [可选 ✨符号] 表示重点任务。

4. 每周完成情况以评论的形式单独在本计划后面给出

## 一、完善 TGOSKITS 仓库

- [x] 【100%】优化 QEMU、board、应用、syscall 和 SMP 测试覆盖 @ZCShou @ZR233 @Lfan-ke @numpy1314 @YanLien @CN-TangLin @Promin3 @nina-ysml @Joshua912815 @seek-hope @Ticonderoga2017 @cg24-THU @crion99 @zyc107109102 @SongShiQ @Antareske @Utopia-V
- [x] 【100%】修复 release-plz、GHCR/rootfs、runner 和 CI 分组问题 @ZR233 @ZCShou @numpy1314 @YanLien @app/github-actions
- [x] 【100%】统一 rootfs/QEMU/vmconfig 与 axbuild target specs 配置 @ZCShou @ZR233 @Jiaxin2006
- [x] 【100%】补齐开发、测试、发布、review 和 reviewer 分配文档 @ZCShou @ZR233 @yks23

## 二、完善 AxVisor

- [ ] 【30%】在 loongarch64 上启动 ArceOS guest，并推进 Linux guest bringup @numpy1314
- [ ] 【60%】完善 x86_64 VMX/SVM、UEFI guest、PIT 和动态平台初始化 @Josen-B @ZCShou @cqwhfhh @ZR233
- [ ] 【60%】修复 AxVisor board CI、riscv guest fault 和 rsext4 recovery 启动问题 @ZR233 @YanLien
- [ ] 【20%】重构 AxVisor host API、vCPU、device 与 irqchip 边界 @ZCShou

## 三、完善 ArceOS

- [x] 【100%】补齐 Rust std app、I/O、threading、Tokio 和 linker 兼容性 @eternalcomet @ZR233 @ZCShou
- [ ] 【70%】完善 lockdep、栈 guard page、affinity、SMP 调度和 work-stealing @shilei-massclouds @ZR233 @yks23 @nina-ysml
- [x] 【100%】完善 smoltcp、ICMP raw socket、VirtIO Net、Vsock 和设备探测 @ZR233 @sunhaosheng @lzaPro @CharlieVinnie
- [x] 【100%】实现 axbacktrace raw report、自动符号化和 QEMU 回归 @Jiaxin2006 @CN-TangLin @shilei-massclouds

## 四、完善 Starry

- [x] 【100%】完成 RK3588 USB/NPU/UVC、Realtek 网卡和 PicoClaw 测例支持 @ZR233 @bullhh @Joshua912815
- [ ] 【30%】补齐 RK3588 OrangePi 5 Plus ttyS1/ttyS3 串口支持 @lianux-mm
- [x] 【100%】完成 SG2002 平台、TPU/ION、USB UVC、ESP ioctl 和 CI build 修复 @ZR233 @pengzechen @BattiestStone4 @yfblock @wyatt-dai
- [ ] 【30%】继续推进 SG2002 board boot 与 K230 KPU QEMU 支持 @bullhh @Joshua912815
- [x] 【100%】修复 futex、robust-list、waitpid、multi-threaded execve 和 signal restore 语义 @yks23 @aptacc2421 @LorenzLorentz @LetsWalkInLine @Ticonderoga2017
- [ ] 【40%】继续处理 clone/vfork、signal delivery、dumpable/no_new_privs 并发语义 @yks23 @seek-hope
- [x] 【100%】补齐 BusyBox、procfs、socket ioctl、UDP/IPv6/UNIX socket 和 OpenJDK 网络兼容 @Promin3 @LorenzLorentz @cg24-THU @zyc107109102 @yks23 @jakeuibn @LetsWalkInLine @Lfan-ke
- [ ] 【40%】继续完善 OpenRC/Git/Redis/MariaDB/nginx、namespace 和 `/proc/stat` 场景 @nina-ysml @aptacc2421 @1301182193 @Antareske @fzg-23 @yks23
- [x] 【100%】补齐 open/openat、uid/gid、prctl、signalfd/eventfd、epoll 和 sync syscall 语义 @Lfan-ke @Joshua912815 @SongShiQ @nina-ysml @aptacc2421 @Utopia-V @Antareske @54dK3n @1301182193
- [ ] 【40%】继续完善 memfd、pidfd、waitid、seccomp、poll/select、execve 和 preadv/pwritev2 语义 @JosephJoshua @aptacc2421 @WellDown64 @CN-TangLin @MuZhao2333 @cg24-THU @foxg1ove1 @irinaparchina-art @Utopia-V
- [x] 【100%】合入 Lua/LuaRocks、Redis、BusyBox、jcode、DeepSeek TUI 和 app runner 测例 @Promin3 @YanLien @SongShiQ @jakeuibn @CharlieVinnie
- [ ] 【50%】继续添加 OpenSSH、GCC、pip、nginx、Git stress、自编译和 qperf harness 测例 @nina-ysml @Ticonderoga2017 @zyc107109102 @Antareske @Utopia-V @cg24-THU @seek-hope @crion99
- [x] 【100%】修复 rsext4 journal/readdir、uninit bitmap、block group 和 axfs-ng rename 语义 @YanLien @ZR233 @yks23 @jakeuibn @aptacc2421 @zyc107109102
- [ ] 【40%】继续完善 ax-fs-ng 直接设备、page reclaim、rsext4 rmdir 和 block cache @yks23 @seek-hope @zyc107109102
- [x] 【100%】完成 SD/MMC platform driver 支持 @YanLien
- [ ] 【40%】继续拆分 CrabUSB、DMA/MMIO API 和共享 driver stack @ZR233
- [x] 【100%】完成 DRM/KMS、Weston bringup、evdev/netlink 和 Apple HVF 图形基础 @JosephJoshua @CN-TangLin
- [ ] 【40%】继续实现 dumb buffer mmap、Xwayland 和 visual regression @JosephJoshua
- [x] 【100%】完成 tracepoint/debugfs、KCOV 调整、GDB ptrace 和内核诊断基础 @Godones @ZR233 @Promin3
- [ ] 【30%】继续推进 kallsyms、memtrack、TCG profiling、eBPF/kprobe/LKM @Godones @Jiaxin2006 @cg24-THU @CN-TangLin @LorenzLorentz

## 五、其他事项

- [x] 【100%】迁移 Sparreal、SomeHAL 和 release metadata 配套内容 @ZR233
- [x] 【100%】完成 SysABI 调研、外部组件评估和月度计划同步 @ZR233
