# 开发计划

1. 阶段 **2026.6.1** ~ **2026.6.30**

2. 任务根据功能分类并以无序列表的形式展现，并在编写时采用 Markdown 的 `- [ ]` 开头以显示复选框，选中复选框表示任务已经完成

3. 任务格式：`【进度百分比】[可选✨符号] 任务名字或者直接写对应的工作项 @负责人`，其中的 [可选 ✨符号] 表示重点任务。

4. 每周完成情况以评论的形式单独在本计划后面给出

## 一、完善 TGOSKITS 仓库

- [ ] 【30%】✨ 梳理工作区目录结构和配置归属，继续收敛平台配置、rootfs 配置、vmconfig、测试配置和生成产物目录，减少 StarryOS、AxVisor、ArceOS 之间的重复维护 @ZCShou @ZR233 @Josen-B
- [ ] 【30%】完善仓库级 CI 和构建测试流程，重点处理 hosted/self-hosted runner 分工、container 环境、缓存策略、失败日志归档、fork 场景和 release 前置检查 @ZR233 @ZCShou @app/github-actions
- [ ] 【30%】统一 QEMU、board、SMP、应用和 syscall 测试的调度入口与分组规则，抽象公共 rootfs 准备、超时控制、结果判定和本地复现命令，而不绑定到单个测试用例 @ZCShou @ZR233 @YanLien
- [ ] 【20%】维护 release-plz 与月度发布流水线，继续处理自动 release、版本元数据、发布分支、crate publish 顺序和 CI 状态门禁之间的衔接问题 @app/github-actions @ZR233
- [ ] 【20%】持续整理文档、组件索引、开发计划和月度总结，确保新增组件、目录迁移、测试矩阵、平台配置和审查流程在文档侧同步更新 @ZCShou @ZR233

## 二、完善 AxVisor

- [ ] 【30%】在 loongarch64 上启动 ArceOS guest，并继续推进 Linux guest bringup，重点处理 vCPU 初始化、timer/IPI、异常注入和启动参数传递 @numpy1314
- [ ] 【60%】✨ 完善 x86_64 VMX/SVM、UEFI guest、PIT 和动态平台初始化，整理 VM 配置到 platform-first 目录结构，并补齐 Linux guest boot 的配置样例 @Josen-B @ZCShou @cqwhfhh @ZR233
- [ ] 【60%】修复 AxVisor board CI、riscv guest fault 和 rsext4 recovery 启动问题，重点覆盖 self-hosted runner、真实板卡 smoke、文件系统恢复和 guest trap 诊断 @ZR233 @YanLien
- [ ] 【25%】✨ 重构 AxVisor host API、vCPU、device 与 irqchip 边界，收敛 ArceOS API 调用面，拆分 architecture setup、IRQ routing、device model 和 VM lifecycle @ZCShou

## 三、完善 ArceOS

- [ ] 【70%】✨ 完善 lockdep、栈 guard page、affinity、SMP 调度和 work-stealing，重点验证抢占、CPU 亲和性、任务迁移和多核 run queue 负载均衡 @shilei-massclouds @ZR233 @yks23 @nina-ysml
- [ ] 【30%】完善 ax-posix-api epoll 等 POSIX 行为，补齐边沿触发、oneshot、ready list、waker 唤醒和 socket/file descriptor 混合场景 @cqwhfhh
- [ ] 【40%】加强 axbacktrace 正确性、分配行为和性能回归覆盖，减少符号化路径中的额外分配，并补充 panic、host backtrace 和裸机 backtrace 测例 @Ticonderoga2017 @Jiaxin2006

## 四、完善 Starry

- [ ] 【30%】补齐 RK3588 OrangePi 5 Plus ttyS1/ttyS3 串口支持，明确 pinmux、clock、interrupt、early console 和 runtime console 的配置路径 @lianux-mm
- [ ] 【40%】✨ 推进 SG2002 board boot 与 K230 KPU QEMU 支持，补齐 KPU driver、devfs 暴露、NNCase runtime demo、镜像准备和 CI smoke 测试 @bullhh @Joshua912815
- [ ] 【50%】继续处理 clone/vfork、signal delivery、dumpable/no_new_privs、ptrace 和 fork/exec/wait 并发语义，重点覆盖 x86_64 ptrace、fork-exec-wait4、multi-thread execve 和信号中断边界 @yks23 @seek-hope @Hoped108 @54dK3n
- [ ] 【50%】完善 memfd、pidfd、waitid、seccomp、io_uring、cgroup2、futex WAKE_OP、poll/select、execve 和 preadv/pwritev2 语义，补齐 Linux errno、阻塞/唤醒、权限检查和用户态兼容测试 @JosephJoshua @aptacc2421 @WellDown64 @CN-TangLin @MuZhao2333 @cg24-THU @foxg1ove1 @irinaparchina-art @Utopia-V @cqwhfhh @LetsWalkInLine
- [ ] 【40%】完善 x86_64 ABI、arch_prctl、child subreaper、mount/umount2 和 fcntl lock deadlock 行为，重点处理 TLS base 校验、进程收养、mount flag 兼容和文件锁死锁检测 @WellDown64 @54dK3n @cqwhfhh
- [ ] 【50%】完善 OpenRC/Git/Redis/MariaDB/nginx、namespace、unshare 和 `/proc` namespace 场景，补齐 pid/user/mount namespace 文件、服务启动脚本、网络工具和多进程 daemon 行为 @nina-ysml @aptacc2421 @Antareske @fzg-23 @yks23 @MuZhao2333
- [ ] 【50%】继续添加 OpenSSH、GCC、pip、nginx、Git stress、curl、apk、glibc/musl 动态链接、自编译和 qperf harness 测例，形成 normal/stress 分组下的稳定回归信号 @nina-ysml @Ticonderoga2017 @zyc107109102 @Antareske @Utopia-V @cg24-THU @seek-hope @crion99 @SongShiQ @irinaparchina-art @foxg1ove1
- [ ] 【50%】✨ 推进 Starry 自编译、loongarch64 to_bin、HVF 自构建应用和 ext4 cache coherence，重点验证 riscv64 8GB rootfs、缓存一致性、链接器输出和多架构 app 打包 @seek-hope @yks23 @Utopia-V
- [ ] 【50%】完善 ax-fs-ng 直接设备、page reclaim、rsext4 rmdir、block cache、xattr、非标准块尺寸和 SMP 细粒度锁，覆盖 tmpfs xattr、非 512B 物理块、非 4K ext4 逻辑块和高并发元数据更新 @yks23 @seek-hope @zyc107109102 @Dirinkbottle @cqwhfhh
- [ ] 【40%】完善 DHCP 生命周期、TCP_INFO、socket option 和网络诊断接口，补齐租约续期/过期、重试上限、`/proc/net/dhcp`、TCP 状态查询和 socket ioctl 兼容 @yydawx @cqwhfhh
- [ ] 【40%】继续实现 dumb buffer mmap、Xwayland、visual regression、HVF SMP 和 macOS 自构建路径，重点验证 KMS buffer mapping、输入事件、Weston/Xwayland 启动和截图回归 @JosephJoshua @yks23
- [ ] 【40%】推进 kallsyms、memtrack、TCG profiling、eBPF/kprobe/LKM 和用户态 eBPF 程序，补齐模块加载、符号解析、probe attach、用户态 loader 和诊断输出 @Godones @Jiaxin2006 @cg24-THU @CN-TangLin @LorenzLorentz

## 五、组件与驱动

- [ ] 【40%】✨ 拆分 CrabUSB、DMA/MMIO API、共享 driver stack 和 DMA sync helpers，明确 coherent/streaming DMA、cache maintenance、iomap 和 OS glue 的边界 @ZR233
- [ ] 【30%】推进 shared IRQ framework、irqchip 接口与跨平台中断注册流程，支持共享中断线、handler 注册/注销、mask/unmask 和 platform-owned routing @ZR233
- [ ] 【40%】完善 K230 KPU driver、devfs 暴露和 NNCase runtime demo，打通设备初始化、内存映射、用户态 ioctl、模型加载和 QEMU/board 测试路径 @Joshua912815

## 六、其他事项

- [ ] 【20%】持续跟踪 open PR review、冲突处理和 reviewer 分配，保持重点任务的审查节奏和合入路径清晰 @ZCShou @ZR233
- [ ] 【20%】同步 6 月周报、月报和项目路线图，按周记录已完成任务、阻塞问题和计划调整 @ZCShou @ZR233
