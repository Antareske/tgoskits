# 2026年5月第四周完成情况

## 一、完善 TGOSKITS 仓库

### 1.1 优化自动测试 @ZCShou @ZR233

- [x] 【100%】✨**解除 release-plz、Starry checks、USB release、axaddrspace std 与首轮 feature push CI 阻塞** https://github.com/rcore-os/tgoskits/pull/648 https://github.com/rcore-os/tgoskits/pull/743 https://github.com/rcore-os/tgoskits/pull/827 @ZR233
- [x] 【100%】✨**优化 CI workflow 输出、自托管 runner 与测试任务配置** https://github.com/rcore-os/tgoskits/pull/832 https://github.com/rcore-os/tgoskits/pull/857 @ZCShou
- [x] 【100%】✨**延长 apk-curl、SMP4、usb-storage 等 Starry QEMU 测例超时时间** https://github.com/rcore-os/tgoskits/pull/785 @Lfan-ke
- [x] 【100%】✨**继续加固 GHCR、rootfs container、apk-curl、USB audio、self-hosted runner 与 release-plz CI** https://github.com/rcore-os/tgoskits/pull/852 @numpy1314
- [x] 【100%】✨**重组 Starry normal QEMU 测例分组** https://github.com/rcore-os/tgoskits/pull/744 @YanLien
- [ ] 【40%】✨**重组 Starry QEMU 测试目录、发现逻辑与 CI 分组选项** https://github.com/rcore-os/tgoskits/pull/1022 @YanLien
- [x] 【100%】✨**支持 syscall grouped binaries 自动执行** https://github.com/rcore-os/tgoskits/pull/835 @ZR233
- [x] 【100%】✨**将 SMP 测例扩展到四种架构并补充并发覆盖** https://github.com/rcore-os/tgoskits/pull/652 @CN-TangLin
- [x] 【100%】✨**添加 Lua / LuaRocks 与 Redis 应用测试覆盖** https://github.com/rcore-os/tgoskits/pull/777 https://github.com/rcore-os/tgoskits/pull/802 @Promin3
- [x] 【100%】✨**添加 app runner 与定时 app smoke CI** https://github.com/rcore-os/tgoskits/pull/812 @YanLien
- [x] 【100%】✨**添加 OpenSSH 应用测例并实现 PR_SET_NO_NEW_PRIVS** https://github.com/rcore-os/tgoskits/pull/810 @nina-ysml
- [ ] 【40%】✨**添加 diffutils 应用测试覆盖** https://github.com/rcore-os/tgoskits/pull/875 @Joshua912815
- [ ] 【40%】✨**推进 riscv64 Starry 自编译应用测试** https://github.com/rcore-os/tgoskits/pull/881 @seek-hope
- [ ] 【40%】✨**添加 GCC 编译测试用例** https://github.com/rcore-os/tgoskits/pull/945 @Ticonderoga2017
- [ ] 【40%】✨**补充 x86_64 Starry 自编译脚本和文档** https://github.com/rcore-os/tgoskits/pull/973 @seek-hope
- [ ] 【40%】✨**添加 syscall 与 qperf 测试 harness** https://github.com/rcore-os/tgoskits/pull/990 @cg24-THU
- [ ] 【50%】✨**添加 apk curl / cmake QEMU 测试覆盖** https://github.com/rcore-os/tgoskits/pull/1000 https://github.com/rcore-os/tgoskits/pull/1017 @crion99
- [ ] 【40%】✨**添加 pip functional 应用测试** https://github.com/rcore-os/tgoskits/pull/1002 @zyc107109102
- [ ] 【60%】✨**迁移 llama.cpp Alpine/musl 兼容性测试到 apps 框架** https://github.com/rcore-os/tgoskits/pull/1006 @SongShiQ
- [ ] 【50%】✨**添加 Alpine nginx 应用 CI** https://github.com/rcore-os/tgoskits/pull/1014 @Antareske
- [ ] 【40%】✨**添加 Git stress 测试套件** https://github.com/rcore-os/tgoskits/pull/1026 @Utopia-V

### 1.2 完善配置系统、文档与发布流程 @ZCShou @ZR233

- [x] 【100%】✨**平台配置处理重构、合并重复 RISC-V 平台实现并移除 `cargo-axplat`** https://github.com/rcore-os/tgoskits/pull/552 https://github.com/rcore-os/tgoskits/pull/833 @ZCShou
- [x] 【100%】✨**使用 target JSON specs 重构 kernel 构建入口** https://github.com/rcore-os/tgoskits/pull/839 @ZR233
- [x] 【100%】✨**更新 GitHub Actions、review workflow、PL031/PL011 文档并增强 clippy 输入处理** https://github.com/rcore-os/tgoskits/pull/608 https://github.com/rcore-os/tgoskits/pull/631 https://github.com/rcore-os/tgoskits/pull/738 https://github.com/rcore-os/tgoskits/pull/739 https://github.com/rcore-os/tgoskits/pull/758 @ZCShou
- [x] 【100%】✨**完善 release-plz 配置、手动发布流程和 crate metadata** https://github.com/rcore-os/tgoskits/pull/620 https://github.com/rcore-os/tgoskits/pull/653 https://github.com/rcore-os/tgoskits/pull/664 https://github.com/rcore-os/tgoskits/pull/745 https://github.com/rcore-os/tgoskits/pull/747 @ZR233
- [x] 【100%】✨**补充 reviewer 重分配、重复 PR 检查与冲突处理流程文档** https://github.com/rcore-os/tgoskits/pull/787 https://github.com/rcore-os/tgoskits/pull/855 https://github.com/rcore-os/tgoskits/pull/858 @ZR233
- [x] 【100%】✨**完善文档结构、组件页面和首页布局** https://github.com/rcore-os/tgoskits/pull/566 https://github.com/rcore-os/tgoskits/pull/578 https://github.com/rcore-os/tgoskits/pull/603 https://github.com/rcore-os/tgoskits/pull/829 @ZCShou
- [ ] 【40%】✨**补充 macOS HVF self-build app 文档** https://github.com/rcore-os/tgoskits/pull/984 @yks23
- [ ] 【80%】✨**跟进 release-plz 本轮 release PR** https://github.com/rcore-os/tgoskits/pull/997 @app/github-actions
- [x] 【100%】✨**修复 axbuild target spec rustflags 配置键名** https://github.com/rcore-os/tgoskits/pull/1023 @Jiaxin2006

## 二、完善 AxVisor

### 2.1 扩展完善 loongarch64 架构支持 @YanLien @numpy1314

- [x] 【100%】✨**启动最小 LoongArch ArceOS guest bringup** https://github.com/rcore-os/tgoskits/pull/768 @numpy1314

### 2.2 扩展完善 x86_64 架构支持 @Josen-B @Ivans-11

- [x] 【100%】✨**添加 AxVisor x86_64 SVM hosted CI 测试任务** https://github.com/rcore-os/tgoskits/pull/701 @Josen-B
- [x] 【100%】✨**为 self-hosted x86_64 测试环境添加 KVM 标签** https://github.com/rcore-os/tgoskits/pull/794 @ZCShou
- [ ] 【40%】✨**添加 x86_64 UEFI guest 支持** https://github.com/rcore-os/tgoskits/pull/760 @cqwhfhh
- [ ] 【60%】✨**增强 SVM Linux guest 支持并改进 PIT 处理** https://github.com/rcore-os/tgoskits/pull/1005 @Josen-B
- [ ] 【40%】✨**支持 x86_64 动态平台初始化** https://github.com/rcore-os/tgoskits/pull/1024 @ZR233

### 2.3 完善板级和 rootfs 启动稳定性 @ZR233 @ZCShou

- [x] 【100%】✨**修复 AxVisor riscv guest memory fault recovery** https://github.com/rcore-os/tgoskits/pull/788 @ZR233
- [ ] 【60%】✨**添加 AxVisor board CI 并加固 rsext4 recovery 挂载流程** https://github.com/rcore-os/tgoskits/pull/830 https://github.com/rcore-os/tgoskits/pull/859 @YanLien
- [x] 【100%】✨**回滚 rsext4 recovery mount 与 AxVisor board CI 改动** https://github.com/rcore-os/tgoskits/pull/838 @ZR233
- [ ] 【20%】✨**重构 AxVisor ArceOS host API 边界并统一模块化入口** https://github.com/rcore-os/tgoskits/pull/1019 @ZCShou

## 三、完善 ArceOS

### 3.2 运行时可靠性与网络栈 @shilei-massclouds @ZR233 @sunhaosheng

- [x] 【100%】✨**实现 axbacktrace raw report、QEMU 回归和自动符号化流程** https://github.com/rcore-os/tgoskits/pull/619 https://github.com/rcore-os/tgoskits/pull/635 https://github.com/rcore-os/tgoskits/pull/646 https://github.com/rcore-os/tgoskits/pull/748 https://github.com/rcore-os/tgoskits/pull/749 https://github.com/rcore-os/tgoskits/pull/793 @Jiaxin2006
- [x] 【100%】✨**将 axbacktrace dwarf 静态可变状态替换为 UnsafeCell** https://github.com/rcore-os/tgoskits/pull/655 @CN-TangLin
- [x] 【100%】✨**为 axtask 添加任务栈 guard page 支持** https://github.com/rcore-os/tgoskits/pull/811 @shilei-massclouds
- [x] 【100%】✨**修复 affinity 更新后的任务迁移逻辑** https://github.com/rcore-os/tgoskits/pull/825 @ZR233
- [ ] 【40%】✨**修复 aarch64 HVF SMP StarryOS 启动** https://github.com/rcore-os/tgoskits/pull/889 @yks23
- [ ] 【40%】✨**修复 lockdep 测试问题** https://github.com/rcore-os/tgoskits/pull/1009 @shilei-massclouds
- [ ] 【70%】✨**优化 axtask select_run_queue 的当前 CPU affinity 偏好** https://github.com/rcore-os/tgoskits/pull/1012 @nina-ysml
- [ ] 【40%】✨**添加 axtask SMP work-stealing 负载均衡** https://github.com/rcore-os/tgoskits/pull/1016 @nina-ysml

## 四、完善 Starry

### 4.1 完善 Starry 在 RK3588 机器人上的支持 @ZR233 @bullhh

- [ ] 【30%】✨**添加 RK3588 OrangePi 5 Plus ttyS1/ttyS3 串口支持** https://github.com/rcore-os/tgoskits/pull/704 @lianux-mm
- [x] 【100%】✨**补充 PicoClaw 机器人场景 smoke 与 gateway 测例** https://github.com/rcore-os/tgoskits/pull/689 https://github.com/rcore-os/tgoskits/pull/775 @Joshua912815

### 4.2 并发卡死 BUG 修复 @shilei-massclouds @seek-hope @LetsWalkInLine

- [x] 【100%】✨**容忍 robust futex cleanup fault** https://github.com/rcore-os/tgoskits/pull/692 @yks23
- [x] 【100%】✨**修正 waitpid child 在 status 写回后的回收时机** https://github.com/rcore-os/tgoskits/pull/686 @aptacc2421
- [x] 【100%】✨**支持 multi-threaded execve** https://github.com/rcore-os/tgoskits/pull/273 @LorenzLorentz
- [x] 【100%】✨**修复 signal restore 跨架构假设并改善 futex / robust-list 基础语义** https://github.com/rcore-os/tgoskits/pull/468 https://github.com/rcore-os/tgoskits/pull/657 @LetsWalkInLine
- [x] 【100%】✨**避免 child-stack clone 场景误触发 vfork wait** https://github.com/rcore-os/tgoskits/pull/693 @yks23
- [x] 【100%】✨**补充 signal delivery 唤醒与 dumpable / no_new_privs 字段** https://github.com/rcore-os/tgoskits/pull/797 @seek-hope

### 4.3 busybox、procfs 与网络兼容性 @wyatt-dai @hongdy22 @Zitao-Chen @sunhaosheng

- [x] 【100%】✨**补齐 BusyBox ifconfig/ifenslave 所需 `/proc/net/dev`、socket ioctl 与 ICMP loopback echo reply** https://github.com/rcore-os/tgoskits/pull/668 @Promin3
- [x] 【100%】✨**补充 BusyBox acpid 与 add-shell 真实路径测试** https://github.com/rcore-os/tgoskits/pull/722 https://github.com/rcore-os/tgoskits/pull/751 @LorenzLorentz
- [x] 【100%】✨**移除非语义 BusyBox 检查** https://github.com/rcore-os/tgoskits/pull/752 @Promin3
- [x] 【100%】✨**集成 qperf profiling 并扩展 BusyBox 测试覆盖** https://github.com/rcore-os/tgoskits/pull/665 @cg24-THU
- [x] 【100%】✨**补充 BusyBox crond 测试并修复 crontab O_TRUNC|O_APPEND 安装路径** https://github.com/rcore-os/tgoskits/pull/741 https://github.com/rcore-os/tgoskits/pull/750 @LorenzLorentz
- [x] 【100%】✨**添加 Git app 与 OpenRC service management 测例** https://github.com/rcore-os/tgoskits/pull/795 https://github.com/rcore-os/tgoskits/pull/845 @nina-ysml
- [ ] 【40%】✨**添加 Redis app QEMU smoke 与 AOF diagnose 测例** https://github.com/rcore-os/tgoskits/pull/808 @aptacc2421
- [x] 【100%】✨**修正 UDP sendto/recvfrom/sendmsg/recvmsg Linux ABI 语义** https://github.com/rcore-os/tgoskits/pull/598 @zyc107109102
- [x] 【100%】✨**支持 v4-mapped IPv6 sockets** https://github.com/rcore-os/tgoskits/pull/694 @yks23
- [x] 【100%】✨**修复 UNIX stream 非阻塞 accept、peer EOF 与 waker 注册** https://github.com/rcore-os/tgoskits/pull/697 @jakeuibn
- [x] 【100%】✨**支持 socket FIOASYNC 状态** https://github.com/rcore-os/tgoskits/pull/796 @LetsWalkInLine
- [ ] 【30%】✨**继续修复 axnet-ng ARP pending buffer、cache TTL 和 jcode TUI no-response 问题** https://github.com/rcore-os/tgoskits/pull/677 https://github.com/rcore-os/tgoskits/pull/681 https://github.com/rcore-os/tgoskits/pull/698 @jakeuibn
- [x] 【100%】✨**暴露 SMP CPU topology 并实现保守的 RISC-V hwprobe** https://github.com/rcore-os/tgoskits/pull/842 https://github.com/rcore-os/tgoskits/pull/843 @yks23
- [x] 【100%】✨**补充 `/proc/self/statm`、`/proc/loadavg` 与 procps 测例** https://github.com/rcore-os/tgoskits/pull/853 @nina-ysml
- [ ] 【30%】✨**支持 MariaDB 应用场景** https://github.com/rcore-os/tgoskits/pull/906 @1301182193
- [ ] 【40%】✨**保持 `/proc/stat` CPU 计数单调递增** https://github.com/rcore-os/tgoskits/pull/941 @yks23
- [ ] 【40%】✨**实现 namespace proxy 与 unshare 基础能力** https://github.com/rcore-os/tgoskits/pull/981 @fzg-23
- [ ] 【40%】✨**修复 nginx multi-worker signal interruption 与 EPOLLEXCLUSIVE 语义** https://github.com/rcore-os/tgoskits/pull/1018 @Antareske

### 4.4 Debian 文件系统和系统调用兼容性 @luodeb @YanLien @CharlieVinnie

- [x] 【100%】✨**修复 sigwaitinfo 等待 blocked signal 卡死** https://github.com/rcore-os/tgoskits/pull/535 @CharlieVinnie
- [x] 【100%】✨**处理 TTY cursor position report** https://github.com/rcore-os/tgoskits/pull/776 @Joshua912815
- [x] 【100%】✨**修复 dup、fcntl 与 flock syscall 兼容性** https://github.com/rcore-os/tgoskits/pull/656 @SongShiQ
- [x] 【100%】✨**添加 uname/sysinfo 覆盖并补充最小 syslog syscall 支持** https://github.com/rcore-os/tgoskits/pull/705 @nina-ysml
- [x] 【100%】✨**补充 anonymous memfd、seals、pidfd 测试** https://github.com/rcore-os/tgoskits/pull/565 @aptacc2421
- [ ] 【70%】✨**完善 memfd seal 语义和 `F_SEAL_WRITE` EBUSY 行为** https://github.com/rcore-os/tgoskits/pull/507 https://github.com/rcore-os/tgoskits/pull/515 @JosephJoshua
- [x] 【100%】✨**修复 open/openat Linux 语义并补齐 grouped syscall 测例** https://github.com/rcore-os/tgoskits/pull/719 https://github.com/rcore-os/tgoskits/pull/720 @Lfan-ke
- [x] 【100%】✨**添加 open/openat 28-module 测试套件** https://github.com/rcore-os/tgoskits/pull/730 @Lfan-ke
- [x] 【100%】✨**校验 sync_file_range flags 与 offset 参数** https://github.com/rcore-os/tgoskits/pull/823 @date727
- [x] 【100%】✨**添加 tmpfs rename exec ELF 回归测例** https://github.com/rcore-os/tgoskits/pull/844 @yks23
- [x] 【100%】✨**补充 uid/gid getter、setter、setres 与 groups syscall 大规模语义测试** https://github.com/rcore-os/tgoskits/pull/725 https://github.com/rcore-os/tgoskits/pull/726 https://github.com/rcore-os/tgoskits/pull/727 https://github.com/rcore-os/tgoskits/pull/728 https://github.com/rcore-os/tgoskits/pull/729 @Lfan-ke
- [x] 【100%】✨**修复 setuid、setfsuid/setfsgid 本地语义缺口** https://github.com/rcore-os/tgoskits/pull/717 @Lfan-ke
- [x] 【100%】✨**修复 PR_SET/GET_DUMPABLE 与 setuid 自动清理 dumpable 语义** https://github.com/rcore-os/tgoskits/pull/718 @Lfan-ke
- [x] 【100%】✨**将 prctl PR_SET_VMA 处理为静默 no-op** https://github.com/rcore-os/tgoskits/pull/780 @Joshua912815
- [x] 【100%】✨**补充 signal syscall 与 basic io syscall 测试** https://github.com/rcore-os/tgoskits/pull/671 https://github.com/rcore-os/tgoskits/pull/784 @1301182193
- [x] 【100%】✨**添加 signalfd4 / eventfd2 测试并修正 signalfd siginfo 字段** https://github.com/rcore-os/tgoskits/pull/683 https://github.com/rcore-os/tgoskits/pull/670 @Utopia-V
- [x] 【100%】✨**添加 epoll syscall 测试** https://github.com/rcore-os/tgoskits/pull/658 @Antareske
- [x] 【100%】✨**重构 syscall 测试并补充 dup2、close_range、ioctl 测例** https://github.com/rcore-os/tgoskits/pull/778 @SongShiQ
- [x] 【100%】✨**添加 select / poll / pselect6 / ppoll 深度测试套件** https://github.com/rcore-os/tgoskits/pull/679 @CN-TangLin
- [x] 【100%】✨**完善 pidfd open/getfd/send_signal Linux conformance** https://github.com/rcore-os/tgoskits/pull/707 @aptacc2421
- [x] 【100%】✨**在 credential 变化时重置 dumpable** https://github.com/rcore-os/tgoskits/pull/757 @cqwhfhh
- [x] 【100%】✨**添加 StarryOS signal extension syscall 回归测例** https://github.com/rcore-os/tgoskits/pull/806 @silicalet
- [x] 【100%】✨**实现 waitid syscall** https://github.com/rcore-os/tgoskits/pull/781 @Joshua912815
- [x] 【100%】✨**添加 prlimit64 用户态测试** https://github.com/rcore-os/tgoskits/pull/801 @WellDown64
- [ ] 【30%】✨**实现 seccomp syscall、BPF filter 与 prctl 接口** https://github.com/rcore-os/tgoskits/pull/1010 @WellDown64
- [x] 【100%】✨**添加 utimensat 测例并修复相关内核缺陷** https://github.com/rcore-os/tgoskits/pull/763 @Utopia-V
- [x] 【100%】✨**实现 sys_sync 与 sys_syncfs 的真实文件系统同步语义** https://github.com/rcore-os/tgoskits/pull/659 https://github.com/rcore-os/tgoskits/pull/660 @Joshua912815
- [x] 【100%】✨**拒绝非法 umount2 flags** https://github.com/rcore-os/tgoskits/pull/699 @54dK3n
- [x] 【100%】✨**保证 MAP_FIXED 失败时保留原映射** https://github.com/rcore-os/tgoskits/pull/691 @aptacc2421
- [x] 【100%】✨**修复 non-ELF executable 通过 `/bin/sh` 重试的 `execve` 兼容性** https://github.com/rcore-os/tgoskits/pull/517 @MuZhao2333
- [ ] 【40%】✨**修正 preadv / pwritev2 Linux 行为** https://github.com/rcore-os/tgoskits/pull/476 @cg24-THU
- [ ] 【30%】✨**完善 StarryOS syscall 测试样例** https://github.com/rcore-os/tgoskits/pull/905 @foxg1ove1
- [ ] 【40%】✨**补充 StarryOS basic syscall semantics 兼容性测试** https://github.com/rcore-os/tgoskits/pull/995 @irinaparchina-art
- [ ] 【40%】✨**补充 rename 回归测例并补齐多架构 test-fchownat 配置** https://github.com/rcore-os/tgoskits/pull/1025 @Utopia-V

### 4.5 文件系统、块设备与 ext4/rsext4 @YanLien @Zitao-Chen @hongdy22 @Ticonderoga2017

- [x] 【100%】✨**保留 rsext4 directory inode generation** https://github.com/rcore-os/tgoskits/pull/828 @ZR233
- [x] 【100%】✨**支持复用 rsext4 uninit inode bitmap** https://github.com/rcore-os/tgoskits/pull/695 @yks23
- [x] 【100%】✨**rsext4 分配块时跳过 inconsistent block groups** https://github.com/rcore-os/tgoskits/pull/675 @jakeuibn
- [x] 【100%】✨**修复 axfs-ng 文件重命名到子目录与 ext4 dentry 删除语义** https://github.com/rcore-os/tgoskits/pull/807 @aptacc2421
- [ ] 【30%】✨**修复 epoll 语义、poll/select/epoll COW fault 与 EPOLLET busy-loop** https://github.com/rcore-os/tgoskits/pull/676 https://github.com/rcore-os/tgoskits/pull/678 https://github.com/rcore-os/tgoskits/pull/696 @jakeuibn
- [x] 【100%】✨**补齐 ax-fs-ng 直接设备传输** https://github.com/rcore-os/tgoskits/pull/800 @yks23
- [ ] 【40%】✨**为 file-backed memory pressure 添加 page reclaim（rebased）** https://github.com/rcore-os/tgoskits/pull/804 https://github.com/rcore-os/tgoskits/pull/1007 @seek-hope
- [x] 【100%】✨**修复 rsext4 非空目录 rmdir 与跨类型 rename 语义** https://github.com/rcore-os/tgoskits/pull/854 @zyc107109102
- [ ] 【40%】✨**替换 rsext4 single-block cache 为 64-entry clock LRU** https://github.com/rcore-os/tgoskits/pull/971 @seek-hope

### 4.6 完善 SD/MMC 驱动 @YanLien

- [x] 【100%】✨**添加 SD/MMC platform driver 支持** https://github.com/rcore-os/tgoskits/pull/826 @YanLien
- [x] 【100%】✨**集成 CrabUSB、vendor dma/mmio API crates，并推进共享 driver stack 拆分** https://github.com/rcore-os/tgoskits/pull/731 https://github.com/rcore-os/tgoskits/pull/742 https://github.com/rcore-os/tgoskits/pull/831 @ZR233

### 4.7 Starry 上 X11 和 Wayland 图形支持 @JosephJoshua

- [ ] 【50%】✨**实现 per-buffer dumb allocation 和 mmap offset key** https://github.com/rcore-os/tgoskits/pull/514 @JosephJoshua
- [x] 【100%】✨**完成 Weston bringup 所需 per-buffer memory allocation 阶段性交付** https://github.com/rcore-os/tgoskits/pull/667 @CN-TangLin
- [x] 【100%】✨**修复 Weston bringup、IRQ wakers、AF_UNIX cmsg byte marks** https://github.com/rcore-os/tgoskits/pull/509 @JosephJoshua
- [ ] 【30%】✨**添加 visual-regression test pipeline 和 Xwayland 场景** https://github.com/rcore-os/tgoskits/pull/516 @JosephJoshua
- [x] 【100%】✨**添加 Apple HVF native execution 所需 GICv3 + CNTV backend** https://github.com/rcore-os/tgoskits/pull/511 @JosephJoshua

### 4.8 Starry 运行时覆盖率与调试 @flying-mice987 @linfeng

- [x] 【100%】✨**添加 Starry kernel tracepoint infrastructure 和 debugfs 集成** https://github.com/rcore-os/tgoskits/pull/673 @Godones
- [x] 【100%】✨**继续跟踪 Starry KCOV 剩余工作、移除 sendfile 忽略产物并调整 QEMU KCOV 测例策略** https://github.com/rcore-os/tgoskits/pull/733 https://github.com/rcore-os/tgoskits/pull/803 @ZR233
- [ ] 【40%】✨**添加 StarryOS TCG hotspot profiling 工具** https://github.com/rcore-os/tgoskits/pull/940 @cg24-THU
- [x] 【100%】✨**添加 kallsyms 内核符号 dump 支持** https://github.com/rcore-os/tgoskits/pull/837 @Godones
- [x] 【100%】✨**添加 kprobe、eBPF subsystem 与 LKM support 基础设施** https://github.com/rcore-os/tgoskits/pull/847 https://github.com/rcore-os/tgoskits/pull/848 https://github.com/rcore-os/tgoskits/pull/849 @CN-TangLin
- [ ] 【30%】✨**添加 riscv64 / x86_64 / aarch64 eBPF JIT backend** https://github.com/rcore-os/tgoskits/pull/891 https://github.com/rcore-os/tgoskits/pull/892 https://github.com/rcore-os/tgoskits/pull/893 @CN-TangLin
- [ ] 【30%】✨**移植 eBPF runtime** https://github.com/rcore-os/tgoskits/pull/850 @LorenzLorentz
- [ ] 【30%】✨**移植 LKM loader / kmod build 流程** https://github.com/rcore-os/tgoskits/pull/851 @LorenzLorentz
- [ ] 【30%】✨**补充 hello / kebpf loadable kernel modules 示例** https://github.com/rcore-os/tgoskits/pull/880 @LorenzLorentz
- [ ] 【30%】✨**移植 aya eBPF userspace programs 与 user-ebpf 构建命令** https://github.com/rcore-os/tgoskits/pull/886 @LorenzLorentz
- [ ] 【30%】✨**添加 memtrack alloc backtrace e2e 测试** https://github.com/rcore-os/tgoskits/pull/1020 @Jiaxin2006

### 4.9 合并 SG2002 开发板支持 @XiaoXiao @elliott10 @ZR233

- [x] 【100%】✨**补充 rockchip-npu workspace metadata** https://github.com/rcore-os/tgoskits/pull/753 @ZR233
- [x] 【100%】✨**添加 SG2002 USB UVC camera 与 ESP-compatible ioctl 支持** https://github.com/rcore-os/tgoskits/pull/791 @yfblock
- [x] 【100%】✨**添加 SG2002 board boot 支持** https://github.com/rcore-os/tgoskits/pull/834 @bullhh
- [ ] 【30%】✨**添加 K230 KPU QEMU 支持** https://github.com/rcore-os/tgoskits/pull/994 @Joshua912815
