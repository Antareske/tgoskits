# StarryOS 网络栈 eBPF 监测跟踪

按「改动 → 测试 → 现象」的周期跟踪推进。方案见 `netmon-plan.md`，结论与验收口径见
`netmon-decision.md`，调研依据见 `netmon-research.md`。

kprobe 原型阶段（历史周期 H1/H2/E1–E3）的方案、跟踪与 PR 草稿已归档到 `archive/`，
本文件不重复其内容，只在新表的"前身"一节留索引。

## 阶段总览

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| 零 | 补安全缺口：故障安全读、加载期有界、ringbuf 可用性 | 已完成（QEMU riscv64，含对照轮） |
| 一 | 把 `NetQueueStats` 接出来（现为 `/sys/kernel/debug/net_queue`） | 已完成（QEMU riscv64） |
| 二 | 静态网络 tracepoint（低频与边界优先） | 进行中：`net:queue_poll`/`queue_irq`/`queue_rearm` 三个栈侧事件在 QEMU 有非零计数，`net:queue_backpressure`/`route_result` 已接通但该负载不触发；两个 wifi 事件待做 |
| 三 | 载体与跨层关联（五个区间） | 已完成（QEMU riscv64）：五个区间各自出分布，采样/不采样两组可比 |
| 四 | 按需 flow / socket 诊断 | 未开始 |

## 周期表

| 周期 | 日期 | 改动 | 测试 | 现象 | 状态 |
| --- | --- | --- | --- | --- | --- |
| P1 | 09-28 | `NetQueueStats` 增加接口名与 poll group id；`NetworkQueueRuntime` 按组记录身份；`ax_net::net_queue_stats()`；`/proc/net/queue`（表头 + 空格分隔） | QEMU：经宿主 HTTP 服务器下载一次，前后各读一次 | 第 1 轮接口名写成 `eth0`，实际发布名是 `virtio-net`，判据不匹配；改为不依赖名字、要求 irq 与 schedule 两个计数在下载前后都增长。第 2 轮：`irq_before=2 irq_after=33 schedule_before=3 schedule_after=68`，运行成功 | 已验 |
| P2 | 09-28 | 组件侧 `ax_net::observe` 单入口 + 默认空操作；内核侧 `tracepoint/net.rs` 定义事件并在 `tracepoint_init()` 装载观察者；`net:queue_poll` | QEMU：单个 netmon 实例跨下载常驻，计数须非零 | 第 1 轮偏移解析错（`field: u32 x offset: 8;` 无分号，解析器把偏移当字段名）；第 2 轮映射到 `;` 段仍取不到偏移；第 3 轮 `queue_poll=78` 通过 | 已验 |
| P3 | 09-28 | `net:queue_irq`（带中断序号）+ `queue_poll` 携带 `irq_sequence_start/end` 区间 | 同上，两个事件计数都须非零 | 第 1 轮 `queue_irq` 偏移写成 16，实际 `u64` 对齐到 24，loader 的 format 核对把加载挡下（这正是它该做的）；第 2 轮两个计数都非零，但判定脚本取错列；第 3 轮 `queue_poll=78 queue_irq=38` 通过 | 已验 |
| P4 | 09-28 | `net:queue_rearm`、`net:queue_backpressure`（reason 用有限枚举 `QueueStallReason`） | 同上，四个事件的计数 | `queue_poll=74 queue_irq=38 queue_rearm=1`；`queue_backpressure=0`——纯下载不会填满 TX ring，事件已接通但该负载不触发它 | 已验（backpressure 未触发） |
| P5 | 09-28 | 区间 1：group 里记"本轮最早一次中断的时刻"，轮询入口取走并算差值 | 同上，`irq_wait_samples` 须非零 | `irq_wait_samples=851`，分布峰值在 2^18–2^20 ns | 已验 |
| P6 | 09-28 | 区间 4/5：`DmaBuffer` 加载体字段；发送侧在交出时打戳、提交时算"队列停留"并覆盖为提交时刻、驱动归还时算"设备持有" | 同上，两个计数都须非零 | 1/1024 采到 0 个样本；速率改 1/64 后 `tx_submitted=37 tx_completed=37`，分布分别落在 2^19–2^24 ns | 已验 |
| P7 | 09-28 | 区间 2/3：接收侧投递时打戳并报"投递发生在轮询的第几微秒"，协议侧取走时算跨 CPU 等待 | 同上 | `rx_enqueued=233 rx_dequeued=233` | 已验 |
| P8 | 09-28 | 采样率做成运行时旋钮（`/proc/net/carrier`），netmon 打印各区间中位桶 | 同一负载跑两次（全采样 / 1/64），中位桶须相差不超过一个桶 | `tx_residence median all=21 sampled=22`——差一个桶，即 log2 直方图的固有分辨率。**（P12 修正：这一轮以及其后各轮里 `echo N > /proc/net/carrier` 都是失败的，两次测量跑的是同一个速率，所以这个对照不成立、这一行不能作为判据 4/7 的证据）** | 已验（对照不成立，见 P12） |
| P9 | 09-29 | `net:route_result`：`RouteDropReason` 有限枚举 + 接口名定长字段；格式解析器改为"取 `offset:` 前面的词"以容纳 `[u8; 16]` 这类含分号的类型 | 同上，`route_result` 须非零 | 接通且偏移核对通过（数组字段当场把旧解析器打回），但访客造不出丢包，计数为 0。查过两条通路：超 MTU 的条件是 `packet.len() > 1500`（IP 包长），IPv4 分片让它不成立；其余 reason 都要求执行器组已消失，QEMU 的 virtio-net 不会 | 已接通，负载与平台到不了 |
| P10 | 09-29 | 慢事件（rearm 等）经 ringbuf 输出明细；ringbuf 拒收时计 `ringbuf_dropped`——观测系统自身的丢失率 | QEMU：慢事件记录数 > 0 且丢失数被报出 | `slow_event_records=1 ringbuf_dropped=0`，记录内容 `kind=queue_rearm device=0 group=0 detail=1` | 已验 |
| P11 | 09-29 | 关闭态逐帧成本：发送侧载体的时钟读取由每帧无条件改为只在采样帧上读（与接收侧一致） | 同一条 QEMU 检查的四个步骤全项通过；并在构建产物上反汇编核对未采样路径 | 全项通过（`queue_poll=629 queue_irq=361 tx_submitted=17 rx_enqueued=74`，`tx_residence median all=21 sampled=22`）；未采样帧的路径上不再出现 `rdtime` 与 `__TimeIf_ticks_to_nanos`，逐帧只剩计数器自增、采样率读、掩码判断与一次载体存储 | 已验（结构核对） |
| P12 | 09-29 | OCR 审查后的修复：有界性检查补上 `BPF_JMP32` 类并在预处理前做结构校验；发送/接收重传路径不再重复上报、不再覆盖载体；`queue_poll` 拆成两个事件以去掉 `allow`；直方图输出 count/sum/mean；`/proc/net/queue` 增加 dev_index 列；字段核对补齐；`prebuild.sh` 补宿主工具准备；`carrier` 写入门控 | QEMU 四步全过（`TRACEPOINTS_LIVE` / `SELFTEST_END` / `QUEUE_COUNTERS_ADVANCED` / kallsyms），`cargo xtask test --since dev` 全过，clippy 两包全过 | 见下『审查修复』一节与其中『比审查报告更严重的一条』：五个区间中位桶（1/4 与 1/16 两组）19/18、19/19、20/20、22/21、19/20，全部在一个桶内；采样点从 17 提到 67+，旋钮写进去后 `carrier_rate` 能读到 4 与 16 | 已验 |
| P13 | — | `wifi:control` / `wifi:sdio_xfer` | 板测（QEMU 镜像里没有 aic8800/SDIO 驱动） | — | 未开始 |
| P14 | — | 撤除 6 处 `#[inline(never)]` | 同负载前后对照 | — | 未开始 |
| P15 | — | 实板：SG2002 + aic8800 | loopback 与设备路径分别验证 | — | 未开始 |
| P16 | — | 按需 LRU flow 表 | 待阶段三数据决定是否立项 | — | 未开始 |

## dev 同步

| 项 | 内容 |
| --- | --- |
| 时间 | 2026-09-29 |
| 变基 | `9a7b868ba`（分叉点）→ `5fd1c6c84`（dev），变基区间比原基线多 33 个提交 |
| 备份 | `backup/sg2002-wifi-ebpf-20260929`，回退用 `git reset --hard <备份分支>` |
| 文本冲突 | 无，20 个提交全部干净应用 |
| 逐提交核对 | `git range-diff`：18 个提交内容完全一致，1 个只有上下文位移，1 个有一处实质差异（见下） |
| 唯一适配 | `NetDeviceError::Dropped` 是本轮 dev 新增的变体，把 `router.rs::route_drop_reason` 的穷尽匹配打破。它只由 TAP 的软件队列产生，上一层 `EthernetDevice` 已把它转成 TX 丢弃计数，到不了观测口，因此归入既有的 catch-all，不新增 payload 值 |
| 锁文件 | 未手改：本分支 `/proc` 与网络栈的改动不引入工作区依赖，`git diff dev..HEAD -- Cargo.lock` 为空 |
| 复核 | `cargo xtask clippy --package ax-net --package starry-kernel`：2 包 89 项全过；`cargo xtask test --since dev`：全部通过；同一条 QEMU 检查四步全项通过（`queue_poll=650 queue_irq=380 irq_wait_samples=371 tx_submitted=17 tx_completed=17 rx_enqueued=74 rx_dequeued=74`，`tx_residence median all=22 sampled=21`，自检 `unbounded=rejected fault=3 bad_ok=0 good_ok=3`） |

本轮 dev 的 33 个提交里只有一个碰到 `net/ax-net`：`17f18f0b4 feat(net): implement TUN/TAP
virtual network devices`。它的三处影响：

1. **TUN/TAP 不经队列运行时。** `create_tap` 直接构造 `EthernetDevice` 包一个 `TapPort`，
   没有轮询周期、中断与 DMA，五个区间在那里无从定义。已列为方案的非目标；路由层的
   `net:route_result` 仍覆盖它们。
2. **`Router::devices` 改为 `Vec<Option<DeviceHandle>>`，设备索引不再复用。** 这对本方案有利：
   事件里带的是设备索引，由 `/sys/kernel/debug/net_queue` 解析成名字，索引不会被重用来指代另一个设备。
3. **`EthernetDevice` 增加 `accept_multicast` 过滤、`NetDeviceError::Dropped` 新增。**
   接收侧的多播过滤发生在帧离开队列之后，`net:frame_enqueue` 统计的是队列发布，不受影响。

`queue_runtime/**`、`drivers/interface/rdif-eth`、`components/ax-tracepoint`、`components/axcpu`
以及 `os/StarryOS/kernel/src/{ebpf,perf,tracepoint}` 本轮都没被 dev 改动。

另有一处迁移影响到本线的文档引用（不是冲突）：`apps/starry/iperf3` 被替换为
`apps/starry/network-throughput`，后者自带 `board-aka-00-sg2002.toml`（等 wlan0 拿到地址后
跑 HTTP 吞吐基准）——正是上板阶段要用的负载生成器。

## 板级准备

出板测镜像要动两处：内核来自本分支，rootfs 借 wifi-opt 线的最新整盘镜像（固件与 init 资产在 p2）。
工具是 `sg2002-image-build`，分两步：`update-kernel` 换 p1 的 `boot.sd`，`inject-rootfs.sh` 注入 p2。

准备阶段发现三件影响镜像正确性的事：

1. **`update-kernel` 不读 `--inject`。** 参数解析是全局的，所以写在命令行上既不报错也不生效，
   症状是"看起来构建成功、镜像里却没有 netmon"。注入必须单独调 `inject-rootfs.sh`，
   以 `"<镜像>?offset=68157440"`（p2 起始字节偏移，即扇区 133120 × 512）指定分区。
2. **不指定 `--dtb` 会顺带换掉 DTB。** 工具用的是 work 目录里当前那份
   `assets/licheerv-nano-sg2002.dtb`，那是上一次构建的残留。核对发现 wifi-opt 树里这份是
   `lcn-sta-q64.dtb`（md5 `c1351ab…`），而基准镜像 attrib3 实际烧的是
   `lcn-sta-defer0.dtb`（md5 `2a82c60…`）——那个树自己的两次产线也不是同一份。
   本轮的取法：内核与 DTB 都用本分支的（本分支是 dev + 观测线，WiFi 走 dev 那一套），
   只借用另一个树里的工具，且从本树根调用使 `--work` 落在本树，不动对方的资产。
3. **netmon 的 riscv64 musl 产物原先不是静态的。** `rustc --print cfg --target
   riscv64gc-unknown-linux-musl` 里没有 `crt-static`，即该目标默认不开静态链接，
   产物 NEEDED `libc.so` 与 `libgcc_s.so.1`，加载要走板子 rootfs 的 musl loader。
   `prebuild.sh` 的头注释一直写着 static，现在按注释显式加 `-C target-feature=+crt-static`。
   这条 rustflags 按目标三元组作用域配置，不会带到 eBPF 那一侧的 `bpfel-unknown-none`。
   产物变为 `statically linked`、0 个 NEEDED，不再依赖 rootfs 提供什么。

板侧联网：sta 模式，SSID `aasta`，密码 `12345678`；WPA2 握手需要熵，起 wpa_supplicant 前
要先备好随机种子。观测线对联网方式本身没有要求，但没有流量就没有样本。

### 本轮镜像

| 项 | 值 |
| --- | --- |
| 镜像 | `sg2002_starryos_wifi_ebpf_netmon_noinitrd_20260929.img`（2.5 GB，sha256 `8f3117f408c5c1eb…`，由 `www/ebpf/build-board-image.sh` 从基准镜像一次产出） |
| 基准 | wifi-opt 线的 `sg2002_starryos_wifi_sta_attrib3_20260929.img`，只借 rootfs |
| 内核 | 本分支 `170c669fd` 的板级构建，**带编译期 STA 凭据**（`STARRY_WIFI_SSID=aasta`）；镜像内取回后核过 SSID 存在且与本树产物逐字节一致 |
| DTB | 脚本每次派生的 `licheerv-nano-sg2002-noinitrd.dtb`：dev 那份删去两个 `linux,initrd-*` 属性，并补 8 个字（32 字节）的 `rng-seed` |
| FIT | 只有 kernel 与 fdt 两个组件（**不含 ramdisk**，见下节根因），config 名与 fdt 节点名仍是 Cvitek U-Boot 认的那两个 |
| p2 初始化 | `/etc/inittab`、`/usr/libexec/starry/console`(0755)、`/etc/profile.d/starry.sh`，取自 dev 自己的 rootfs（见下节第二条根因） |
| 注入 | `/usr/bin/netmon`（静态 musl riscv64，sha256 `f113957c5ef68adc…`）、`/usr/bin/netmon-board.sh`（sha256 `ca2b61ce0e385d89…`），均 0755 |
| 复核 | 内核与 DTB 的哈希与 FIT 记录一致；从镜像取出 DTB 反编译确认 `/chosen` 无 initrd 属性且含 `rng-seed`；三个初始化文件与注入的两个负载均从 p2 取回逐字节比对一致 |

先前那一版 `sg2002_starryos_wifi_ebpf_netmon_20260929.img` 的 FIT 带 ramdisk、DTB 又声明 initrd，
**启动即 panic**；随后一版解决了 panic 但没有 shell（getty respawn 死循环）。两次的根因见下节。

板侧脚本本轮改了两处：默认监测程序路径从 `/tmp/netmon` 改为 `/usr/bin/netmon`（与注入位置一致，
原先按文档那条命令跑会立刻报"不是可执行文件"）；负载一个字节都没传回时直接失败，
不再报成一次"很慢的运行"——这跟 QEMU 里那三次假通过是同一类症状。原先打印的 `seconds=`
是个未被使用的预估（实际时长由负载文件大小决定），已删除，改为低于 5 秒时打印 warning。

### 应用侧验证

改完 `prebuild.sh`（静态链接）与计数器的检查步骤后，`cargo xtask starry app qemu -t ebpf/netmon
--arch riscv64` 重跑得到真实 `EXIT=0`（不接管道），四步完成标记齐全：五个区间在两个速率下
完全一致（`irq_wait 19/19`、`rx_publish 19/19`、`rx_residence 20/20`、`tx_residence 22/22`、
`tx_device 19/19`），自检 `unbounded=rejected fault=3 bad_ok=0 good_ok=3 steps=0`，
`ringbuf_dropped=0`。静态产物在 StarryOS 里照常加载、附着并产出直方图。

顺带澄清一个读数：某一快照出现过 `tx_submitted=63 tx_completed=64`。**这不是缺陷**——
两个计数器读自 eBPF 的 per-CPU map，随 netmon 进程从零开始，而帧上的标记在内核侧，
所以"监测启动前已投递、启动后才完成"的帧只会在完成侧被计一次。检查步骤跑两个速率就是两个进程，
切换时在途的帧正好造成这种差一。板测判据已据此放宽，不再要求单向不等式。

## 板上启动链（三次迭代）

上板冒烟本身要解决三层与观测线无关的启动问题。三层都在代码与提交里核过，按发现的顺序记：

### 一、initramfs panic

最初的表现：

```
ARCEOS_PANIC_EMERGENCY
panicked at os/arceos/modules/axruntime/src/fs/block.rs:331:41:
host initramfs unpack failed: Corrupt("NUL in symlink target")
```

- dev 今天 15:24 合入 `fb3edd5cf feat(initramfs): unify host image boot flow (#2528)`，
  它把 FDT 里 `linux,initrd-start/end` 指向的区间**当 host initramfs 解包**，解不开就 panic
  （`axruntime` 里 `unwrap_or_else(|e| panic!(...))`，someboot 负责按 FDT 预留该区间并 publish）。
  该 PR 里有一个子提交 `fix(ax-fs-ng): reject NUL symlink targets in initramfs` —— 我们撞上的
  `Corrupt("NUL in symlink target")` 就是它刻意加的校验。
- 那个区间**不是 U-Boot 临时给的，而是 DTB 自己声明的**：
  `os/StarryOS/configs/board/licheerv-nano-sg2002.dtb` 的 `/chosen` 里写死了
  `linux,initrd-start = <0x87e81000>` 与 `linux,initrd-end = <0x880ea1a0>`
  （官方 Linux 那套流程里这个地址恒定放 ramdisk，DTB 也就照抄了）。
  U-Boot 日志里的 `Loading Ramdisk to 87e81000, end 880ea1a0` 与这两个值重合，
  所以第一轮把它当成了 U-Boot patch 的结果——其实属性本来就在。
- 于是这段地址上"恰好放着什么"决定了报错：第一次启动 FIT 的 config 带
  `ramdisk = "ramdisk-1"`，U-Boot 把官方 Linux 镜像里的 Cvitek 平台 ramdisk 放上去，
  内核解出 `Corrupt("NUL in symlink target")`；把 FIT 的 ramdisk 去掉后那个地址换成了别的东西
  （DTB 本身加载在 `0x87e7800`，紧邻其后），于是同一处变成 `UnsupportedCompression`。
  两次都证明区间来自 DTB，与 FIT 无关。
- wifi-opt 线不炸只是因为它的 HEAD 不含 #2528（已用 `merge-base --is-ancestor` 确认），
  内核里连这段代码都没有。它的自定义 DTB（`lcn-sta-defer0` 等）同样写死这两个属性，
  所以**一旦变基到 dev，用同一套工具出的镜像会以同样的方式 panic。**

### 二、没有 shell：getty respawn 死循环

initramfs 解决后内核起来了，但控制台被刷屏、拿不到 shell：

```
init: can't open /dev/tty5: No such file or directory
init: process '/sbin/getty 38400 tty1' (pid 365) exited. Scheduling for restart.
```

- 根因链：`fb3edd5cf` 把 `legacy-board-init` 从三个 SG2002 板级配置里一并删掉了
  （该 feature 在代码里也已不存在）。有这个 feature 时是内核自己起 console shell，
  绕过 rootfs 的 init；去掉之后内核走 rootfs 的 `/sbin/init`。
- 而这张镜像的 p2 里是**原版 Alpine 的 inittab**：`tty1..tty6` 各一条
  `respawn:/sbin/getty`。StarryOS 的 devfs 只提供 `/dev/ttyS<N>` 与 `/dev/tty`（控制终端），
  **没有虚拟控制台 `/dev/tty1..N`**，所以这六条永远起不来、永远 respawn。这是结构性的，不是配置问题。
- dev 自己的 rootfs（`cargo xtask starry rootfs` 产出，即本树
  `target/axbuild/rootfs/rootfs-riscv64-alpine.img`）里的 inittab 是 StarryOS 专用的：
  openrc 三条之后是 `::respawn:-/usr/libexec/starry/console`，没有那六条 getty。
  那个 console 助手 source `/etc/profile.d/starry.sh`（其中 `PS1='\u@\h:\w\$ '`）后
  `exec /bin/sh -l -i` —— 也就是 harness 的 `shell_prefix = "root@starry:"` 等的那个提示符。
  我们这张镜像的 p2 沿自另一条镜像链，这三样东西都没有。

### 三、WiFi 的启动熵

- 他们自定义 DTB 的 `/chosen` 里比 dev 那份多一项 `rng-seed`（8 个字 = 32 字节）。
  内核确实读它：`someboot` 把 `/chosen/rng-seed` 当启动熵来源
  （`platforms/someboot/src/entropy.rs`），且要求**恰好 32 字节**。
  这正是 WPA2 握手需要先备好随机种子的原因，dev 那份 DTB 没有这一项。

### 四、WiFi 不关联

shell 拿到之后 `wlan0` 是 `UP`、`LOWER_UP`，但 `ip -4 addr show dev wlan0` 没有地址，也没有流量。

- **关联是编译期做完的，不在用户态。** `drivers/ax-driver/build.rs` 读 `STARRY_WIFI_SSID` /
  `STARRY_WIFI_PASSWORD`，`src/net/aic8800/startup_config.rs` 用 `option_env!` 把 SSID 与
  由 PBKDF2 算出的 WPA2 PMK 编进驱动，关联由驱动自己发起。**rootfs 里没有 wpa_supplicant
  不是问题**——本来就不需要（官方 Linux 镜像那套 `/etc/init.d/S30wifi` +
  `wpa_supplicant -B -i wlan0` 是另一套用户态方案，与 StarryOS 无关）。
- 缺凭据的症状正是"内核照常起、`wlan0` 也 UP，但永远不关联、拿不到地址"。
  wifi-opt 线的复现步骤里写明了这一点：`stage-delivery-20260929.md` 的构建命令带
  `AIC8800_FIRMWARE_DIR=<固件缓存> STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678`，
  并注明"缺凭据内核能起但不关联，取错 SSID 会因关联被拒而 panic"，出镜像前用
  `strings starryos.bin | grep -c aasta` 自检。
- 固件那半一直是对的：`drivers/net/aic8800/build.rs` 把固件从固定提交的仓库（或
  `$AIC8800_FIRMWARE_DIR` 本地缓存）provision 进 OUT_DIR 再编入内核，与 rootfs 的
  `/lib/firmware` 无关。本树那次构建已经拿到 20 个固件文件。
- 我们的构建漏了凭据这一半，于是白烧了一次。
- 另一件相关的事：上游的启动熵注入 `scripts/axbuild/src/starry/boot_entropy.rs::
  prepare_for_secure_wifi`（当构建环境给了 SSID+口令时，用主机随机数生成 32 字节
  `/chosen/rng-seed` 写进**临时 DTB 副本**）**只接在 `starry run` 板卡运行路径上，
  普通 `cargo xtask starry build` 不调用** —— 所以裸构建 + 烧写镜像这条路上，熵与凭据
  两半都得自己给。wifi-opt 线那边也记录了这一点。

### 处置

出镜像用 `www/ebpf/build-board-image.sh`，它把下面四件事一次做完并逐项回读校验；手工多步拼装
出过岔子，所以固化成一条命令。它要求四个输入：基准镜像、输出路径、工具目录，以及
`STARRY_WIFI_SSID` / `STARRY_WIFI_PASSWORD`（缺了就拒绝出镜像——这正是上面第四条那个坑）。

1. 派生 `licheerv-nano-sg2002-noinitrd.dtb`：在 dev 那份基础上 `fdtput -d` 删掉两个
   `linux,initrd-*` 属性，并 `fdtput -t x` 补上 8 个字的 `rng-seed`（每次新取的随机数，
   不照抄他们的）。反编译对比过：只少那两行、多这一行。
   `someboot::initramfs_from_fdt` 在两个属性都不存在时返回 `None`，内核就不走解包这条路，
   改走 SD 卡 rootfs（`root=/dev/mmcblk0p2`）。
2. `www/ebpf/boot-sg2002-noinitrd.its`：FIT 里不放 ramdisk，使那段地址不再是任何东西的落点。
   模板保留了 Cvitek U-Boot 认的 config 名与 fdt 节点名、内核 load/entry 与哈希算法；
   `repack-fit.sh` 认 `ITS` 环境变量，所以不必改那个工具。
3. p2 补 `/etc/inittab`、`/usr/libexec/starry/console`(0755)、`/etc/profile.d/starry.sh`，
   三者都取自 dev 自己的 rootfs，逐字节比对过。

这三层里的第 2、3 层都是 dev 侧今天这个提交带来的板级回归（第 2 层还叠着镜像链自身的
rootfs 差异），值得单独提给项目；本分支先用上面这份可复现的镜像绕开。

## 板上首轮完整测量

四组跑完（`NETMON_BOARD_END`，无 `GROUPS_FAILED`），监视器按组正确退出（日志两条
`Task(…, "netmon") exit with code: 0`）。

### 吞吐（kbit/s）

| 组 | 本端 | 对端（服务端数的） |
| --- | --- | --- |
| `off` | 7390 | 7390 |
| `observe`（速率 0） | 1940 | 1970 |
| `sampled`（速率 16） | 1940 | 1920 |
| `off2`（对照） | 4300 | 4290 |

三条读法：

1. **打戳是免费的**：`observe` 与 `sampled` 的字节数**完全相同**（4844421），速率同为 1940 —— 速率 16
   的逐帧打戳没有可测到的代价。
2. **附着观测者代价很大，而且是确定性的**：两次带监视器的运行给出**同样的字节数**（不是漂移的形状），
   而两次不带监视器的运行相差很大（7390 对 4300）。即观测成本落在"附着"这一侧
   （常开事件：每轮询一次的 `queue_poll`、每次中断一次的 `queue_irq`），不在逐帧采样那一侧。
3. **板子自身吞吐会掉**：`off` 7390 → `off2` 4300（同为 20 s、同为无监视器）。所以验收数字本身有
   这么大的不确定度，而"带监视器更低"要在这么大的漂移背景下读。要更干净地分离，应把带/不带监视器的
   组交错（off, observe, off, sampled, off）而不是先做完全部对照组。

### 分布（`sampled` 组，速率 16）

`intervals irq_wait=17 rx_publish=13 rx_residence=21 tx_residence=23 tx_device=26`（五个中位桶都 ≥ 0）。

| 直方图 | count | mean | 形状 |
| --- | --- | --- | --- |
| `tx_device` | 86 | 135.2 ms | 集中在桶 25–27（33–268 ms），尾部到桶 30 |
| `tx_residence` | 89 | 13.8 ms | 桶 17 起，峰在桶 23（8 ms）、右侧拖到桶 25 |
| `rx_residence` | 23 | 5.5 ms | 集中在桶 21–23 |
| `rx_publish` | 23 | 17 µs | 集中在桶 13（8 µs），少量散到 16 |

设备那一段（`tx_device` 均值 135 ms）是 TX 侧最大的一段，与 wifi 线历来的结论一致。
`observe` 组（速率 0）如设计所料：`carrier_rate=0`，逐帧直方图全空、`tx_submitted=0`、`rx_enqueued=0`，
只有常开指标在动（`queue_poll=986 queue_irq=1195`）。

### 事实来源与待查

会话末尾的 `/sys/kernel/debug/net_queue`：

```
irq 196266  schedule 265149  missed 146559  poll_batches 266909
budget_exhaustion 0  spurious 1  probe_deferred 0  rearm_race 223937
```

`budget_exhaustion` 与 `probe_deferred` 都是 0，所以"每到一轮询预算就被用光"不是吞吐被钉住的机制；
`missed` 与 `rearm_race` 都是十万量级，值得单独查。这些计数是整场累计的，要归因得**按组采样**
（每组前后各取一次），而不是只在末尾取一次 —— 这是下一轮该加的。

## 范围调整：WiFi 层移出

WiFi 层正在别处重构，形状会变，因此本分支的交付面收敛为 **StarryOS 网络栈**：
`net:route_result`、`net:queue_*`、`net:frame_*`、`net:driver_*` 这一组事件与五个区间。
`wifi:control` / `wifi:sdio_xfer` 两个事件**不做**，`netmon-plan.md` 的阶段二与"要上板的是两类"
两处已按此更新。

由此产生两个后果：

- 原先"只有三类必须上板"里排在第一位的那条（wifi 事件只能取到板上）消失，板上剩下的两件是
  板子特有现象与**探针开销量化** —— 后者已于本轮取到第一版数字（见上节）。
- 应用里仍留着**原型阶段**的 SDIO/WiFi kprobe 路径（`netmon-common` 的槽位、`netmon-ebpf` 的
  程序、用户态的探针表与输出行，约 65 处）。它们是 optional、缺符号即跳过、QEMU 里也不跑，
  所以不影响任何验证；但既然指向的驱动形状要变，收掉更合范围。收掉是一次机械改动
  （三个文件 + README + QEMU 的符号检查），代价是重编重验（板测若要重跑则还要一次烧写），
  因此先按现状保留、待明确后再动。

## 审查修复（OCR 第 1 轮）

审查会话 `2026-09-29-sg2002-wifi-ebpf`，六位评审者，结论 REQUEST CHANGES
（3 blocker / 9 should-fix / 12 suggestion）。处理口径：先让这条线正确，顺手补强，
不为"关闭态周期数""故障注入"这类只有上板才有意义的事情投入。

**三个 blocker**

1. `verify.rs` 的有界性检查只匹配 `BPF_JMP`（0x05），整类 `BPF_JMP32`（0x06）被跳过。
   已按类别判定，两个跳转类都走同一条检查；并补了 class-6 的向后分支夹具
   （修复前该夹具必然失败）。
2. 预处理器在 `insns[0] == LD_DW_IMM` 且只有一条指令时越界 panic，而这发生在检查之前。
   新增 `check_structure`，在预处理之前校验原始指令流（长度、宽加载的操作数槽）。
3. 重传路径把载体当成两个语义用：提交前重新打戳，失败后带着"本次尝试时刻"回到队列，
   于是同一帧被上报两次、队列停留被算成"两次尝试之间"。现在失败的尝试把载体恢复成交出时刻，
   所以每一份样本都从交出时刻量起；`tx_submitted` 计的是"向设备的投递次数"，
   在背压下可以大于 `tx_completed`，这一点已写进 README。接收侧同理：
   发布只在确认队列有空位之后才做决定，重传不会重复计数、也不会挪动时间戳。

**修复过程中发现的、比审查报告更严重的一条**

`/proc/net/carrier` 从写入的第一天起就写不进去：`echo N > file` 用 `O_TRUNC` 打开，
而这条路径在 VFS 里实现成一次零长度写入，写处理函数把空输入当成非法输入拒掉，于是
**open 本身就失败**（`/bin/sh: can't create /proc/net/carrier: Invalid argument`）。
同一个文件里主线的 `/proc/sys/kernel/hostname` 正是用 `if data.is_empty()` 处理这一步的，
本分支抄了它的形状却漏了这一句。

后果不止一个报错：脚本里两次 `echo` 都失败，**采样率从未真正改变**，因此 P8 的
"全采样 vs 1/64"、判据 4 的"采样不改变分布"、判据 7 的"两组可比"此前都没有被验证过；
它一直没暴露，是因为 `fail_regex` 不匹配这条 shell 报错，而"中位桶相差一个桶"在噪声内也讲得通。
修复即补上那句短路，并把 QEMU 的两组改成 1 与 1/16（采样点从 17 提到 71，中位数才站得住）。
修好后五个区间中位桶分别为 20/19、20/19、21/20、20/21、21/20——全部在一个桶内。

**落点的修正（审查报告之后的追加）**

`/proc/net/{queue,carrier}` 是 Linux 已占用的命名空间：那个目录是网络栈自己的统计面，
`dev` 的表头我们逐字对齐过 `dev_seq_show()`，往里加 Linux 没有的名字等于在这条兼容性契约上开口子。
`carrier` 还占用了 Linux 表示链路载波的词。tgoskits 的既有惯例是：Linux 没有命名的诊断面放
**debugfs** 并做特性门控（`/sys/kernel/debug/{scheduler_metrics,file_lock_metrics}`，
PR #1775/#2313/#2451 与 #2491，其中 #2491 的提交信息自己写着"expose state and record capacity in debugfs"）；
而 `sysfs.rs` 明写可写 sysfs 旋钮不在范围内。`/proc` 里仅有的两个自定义名字
（`instret`、`meminfo2`）是老单体仓库的遗留，一行提交信息、无注释、无评审记录，不是榜样。

因此两个条目搬到 `/sys/kernel/debug/`：`net_queue`（只读，每个 poll group 一行）与
`net_sample_rate`（可读写，取代 `carrier` 这个名字）。特性门控这次不做——和它们配套的
`net:*` tracepoint 本来没门控，门控这个文件会让"设备索引怎么解析"随构建配置而变。
这一条与兄弟条目的惯例不一致，若 reviewer 提出再补。

**其余修复**（按影响排序）

- `queue_poll` 拆成 `queue_poll`（duration/work/blocked）与 `queue_poll_irq`
  （irqs_absorbed/irq_wait_ns）两个事件。tracepoint 宏按字段数生成函数，九字段一条记录
  过不了参数个数的 lint；拆分之后 `#[allow(clippy::too_many_arguments)]` 撤除，
  两个问题也各自可读。
- 每个直方图输出 `count` / `sum_ns` / `mean_ns`：sum 槽排在桶块之后、同序，
  于是 log2 桶丢掉的和可以还原成均值，count 就是实际采样数。`carrier_rate` 一并输出，
  判据五由此达成。
- 四个 tracepoint 处理函数里"值为零就丢弃"的写法统一：只有表示"有没有"的字段才设门，
  表示"多少"的字段零也是样本。区间一的分布不再被系统性推高以外的原因削薄。
- 字段核对补齐：`queue_rearm` 的三个字段都用 `netmon-common` 的常量列出；
  `TracePointSpec` 上注明"程序读的字段必须列全，多列是刻意的布局锚点"。
- 计数表增加 `dev_index` 列（追加在行尾，不打乱既有列序；其所在文件后来搬进 debugfs），
  事件里带的设备索引因此有了解析出口——此前 `NetQueueStats` 根本没带这个字段，
  而 `tracepoint/net.rs` 声称靠这张表解析。
- `netmon/prebuild.sh` 补上兄弟应用都有的 `rustup target add` 与 bpf-linker 准备
  （0.11.1），干净机器上 README 里那条命令不再在 build script 里 panic。
- 采样率旋钮的写入加 `euid != 0` 门控；同一个文件里已有的写法（该旋钮后来搬进 debugfs 并改名 `net_sample_rate`）。
- 收尾：`route_name` 宽度改为引用 `ax_net::observe::ROUTE_NAME_LEN`；`net_queue_irq`
  不再读取后丢弃序号；`PollGroupState` 的文档注释复位；README 的采样口径与输出格式
  重写（原先"计数器统计每一次事件"对四个采样计数器是假的）。

**不做**（判断依据见下）：`bpf(2)` 权限门禁（既有子系统属性，单用户研究系统）；
`route_result`/`queue_backpressure` 的故障注入（为观测而改造网络栈，方向反了）；
观察者卸载路径（框架级，且"off 组"可在板上用 tracefs enable 得到）；
撤除六处 `#[inline(never)]`（板上冒烟还要用）。

## 阶段零的对照轮

按测试规则，自检必须先证明"在有缺陷的实现上必然失败"，再证明修复后通过。
第 3 轮把 helper 表恢复成 kbpf-basic 的原始实现（其余不变），跑同一条自检，
预期在内核里出现未处理的页错误；这一轮的结果见下表。

| 轮次 | 改动 | 现象 |
| --- | --- | --- |
| 对照 | helper 4 恢复为 kbpf 的裸 memcpy（`verify` 与其余改动保留） | 内核 panic：`Unhandled Supervisor Page Fault @ 0xffffffff80598c9e, fault_vaddr=VA:0xfffffffffffff000 (READ)`，harness 以 `exit=1` 判失败。同一份自检在修复后全项通过 |

## 阶段四：protocol 侧与 flow 表

### A. protocol executor 的饱和信号（`net:proto_poll`，QEMU 已验证）

落点是 `poll_protocol_until_idle` 的那次 `get_service().poll(...)`：每次轮询发一条事件，
带时长、`more`（还有事做）、`yielded`（这次轮询把连续轮询预算用尽，之后让位给设备 owner）。
eBPF 侧按 `more`/`yielded` 分别计数，时长进直方图。QEMU（`/tmp/qemu-r8.log`）：

```
proto_poll=1800 proto_poll_more=914 proto_poll_yielded=459
```

即 25% 的轮询把预算用尽。这是"protocol executor 是否饱和"的直接答案，
与队列侧 `queue_poll` 同形。

### B. flow 表（`net:flow` + LRU map，待 QEMU 验证）

形状：内核在 socket 调用点发事件（key = 本侧视角的端点对，40 字节定长；
kind = open/close/tx/rx；bytes；该 socket 当时的 RTO 微秒）；eBPF 侧把事件累加进
一张 LRU hash 表（128 项，value 48 字节）；userspace 用 `--flows N` 打印最忙的几条。

实现要点（都是踩过的坑）：

- **端点取自 socket 自己记录的那一份**（`bound_endpoint` / `peer_endpoint`），
  不是 smoltcp 的实时 tuple：不需要 SOCKET_SET 锁，而且连接结束后 tuple 消失时
  仍然报得出 close。
- **close 只在 `Drop` 报一次**：每个 socket 对象必然经过它，而 `shutdown` 之后再
  drop 会重复计数。
- **peek 不计 rx**：`MSG_PEEK` 不消耗数据，计进去就等于把同一批字节数两遍。
- **eBPF 侧读改写而不是原地改**：`lookup` 返回的是 map 内部存储的指针，
  释放 map 锁之后另一 CPU 的淘汰可能把它 free 掉；改成取出→改栈上副本→`insert` 回写，
  代价是多一次哈希插入，换来不会踩到已释放的值。丢更新仍在（同 key 并发），
  单核精确、多核可能略少，已写进文档。
- **key 带版本字节**：解码方遇到不认识的版本拒绝解码，而不是把字节当地址渲染出来。
- **内核只加载"调用全是 helper"的程序**（`NonHelperCall { index, source }` 拒载）。
  eBPF 侧原先写成"取出整条 value → 改 → 写回"，44 字节的整体拷贝被 LLVM 降成
  `memset`/`memcpy` 库调用（反汇编里是 `call -0x1`），于是 `net_flow` 整个程序加载失败——
  现象是 monitor 起不来，harness 只看到 `Error: the BPF_PROG_LOAD syscall returned
  Invalid argument`，而真正的原因在内核日志的 `bpf prog rejected:` 那一行。
  改法：读的时候按字段读、写的时候用算好的字段构造 value，全程不出现整体拷贝。
  排查手法已写进 README：`llvm-objdump -d --section=tracepoint <obj>` 列出所有 call，
  `call -0x1` 就是要被拒的那个。

QEMU 侧的检查（`netmon-check flows`）：一次运行内跑下载，断言表里最忙的一条
有非零字节、且 `flow_events` 计数非零。表在监测进程自己的 map 里，
所以必须由填表的那个进程打印。

**QEMU 验证结果**（通过的那一轮）：

```
flow tcp 10.0.2.15:49155->10.0.2.2:18384 tx_bytes=90 tx_ops=1 \
     rx_bytes=4194367 rx_ops=132 opens=1 closes=1 last_kind=2 rto_micros=0 last_ns=25224246500
```

对得上事实：本端 10.0.2.15、对端 10.0.2.2:18384（harness 的 HTTP server），
`tx_bytes=90` 就是那一条 HTTP GET（一次 send），`rx_bytes=4194367` 是 4194304 字节正文
加响应头，`opens/closes=1`、`last_kind=2`（close）与 wget 建连、退出各一次吻合。
即 key 的布局、解码、字节归集、生命周期四处都对上了。
`rto_micros=0` 是正常的：`socket.timeout()` 只在重传定时器armed时给值，健康连接上一直是 0。

同一轮里 `flow_events` 的量级很小：一次 4 MB 下载约 135 条事件（`flow_rx=135`），
即本端一次 recv 就吃掉几十 KB。这一条也要记住：flow 事件是**每次 socket 调用**一条，
不是每帧一条，所以它在内核侧的开销与分配量都很小。

### 顺带修掉的两处检查机制问题

- `netmon-check` 失败时现在打印 `NETMON_CHECK_FAILED ...`，case 的 `fail_regex`
  也加了这条：runner 只有在输出命中 `fail_regex` 时才立刻结束该步，
  否则会一直等到整体 timeout（600 s），并把后面的检查步骤全部拖没。
- `rates` 检查原先比两个采样率的中位数、容差 1 个桶。实测四轮里有一轮
  `irq_wait` 差了 2 个桶（20 vs 18）——两次运行是两次独立的下载，样本数本身差 10%
  （336 vs 371），中位数因此会动。改为：① 加一组速率 0，断言逐帧直方图必须为空
  （这是"旋钮决定是否打时间戳"的结构性判据，与负载无关）；② 两个采样率的比较容差放到
  2 个桶并写明理由（要抓的是分布整体平移，不是一次下载带来的抖动）。

## 测试机制备忘

- `qemu-*.toml` 的 `success_regex` 语义是**任一命中即通过**，不是全部命中。
  要断言多行同时成立，只能把它们写成一条有序的多行正则（`(?s)` + `.*`），
  否则某一行不成立时这一步仍然"通过"。
- 第 1 轮就因为这个语义而假通过了一次：`unbounded=accepted` 与
  `unbounded=rejected` 相矛盾，运行却报成功。
- `shell_check_steps` 的 `shell_cmd` 可以用 TOML 多行字面量写，**但每行都得短**：
  终端 80 列，而提示符 `root@starry:~# ` 占掉 15 列，所以输入行的实际上限约 64 列。
  超出的部分被折行，折出来的后半段丢掉行首的 `#`、或者展开成一条独立命令送进 shell。
  两轮都栽在这上面：一次是 76 列的注释，一次是 `[ $ok -eq 1 ] && ... || exit 0` 折成
  `$w -gt 0 ] || exit 0`，而 `$w` 展开成 89（恰好是 `irq_wait_samples` 的值），
  于是 shell 去找一个叫 `89` 的命令。两次都撞上 `fail_regex` 里的 `/bin/sh: .*not found`。
- 折行和输出的交错**有竞态**：同样 70 列的一行，上一轮没炸、下一轮炸了。所以不能按
  "刚好塞下"给预算，现在整段按每行不超过 58 列写（提示符 15 + 58 = 73，留 7 列余量）。
- 比长度更坑的是**复合语句**。`for ... do ... done` 会让 shell 停在等续行的状态，
  而这条串口链路分不清"等续行"和"命令已结束"——于是循环体被揉进 echo 的输出里，
  只打出一行 `median irq_wait rate4=20 irq_wait rate16=19 irq_wait`。
  加不加反斜杠续行都一样，说明问题出在 `for` 这个结构本身；函数定义是好的
  （`run_load`/`measure`/`fn`/`med` 一直正常）。改成"一个函数 + 五次顺序调用"之后就正常。
- 上面那次揉坏直接造成了**一次假通过**：断言根本没执行，`ok` 停在初值，
  于是 `TRACEPOINTS_LIVE` 照样打出来、harness 报 `exit=0`，
  而唯一漏出来的那行数据 `rate4=20 rate16=19` 其实差了两个桶、本该失败。
    同一个坑的另一面：`success_regex` 是"任一命中即通过"，所以"打出了完成标记"这件事本身
  不证明断言跑过——**只有把中间量也打出来、并且眼看过**，才知道判定逻辑真的执行了。
- 第三次假通过是**我自己造的**：把 `fn`/`med` 从一行改写成两行时丢了 `tail -1`，
  于是取回的是"所有快照里匹配到的行"拼成的多行串，`cut` 之后成了 `19 irq_wait` 这种畸形值；
  断言拿两个**相同的畸形值**相减得 0，照样通过。教训：提取函数必须取"一个值"而不是"一组值"，
  而且判定要打印带定界符的原值（现在写成 `r4=[$a] r16=[$b]`），
  否则杂质的唯一症状就是"看起来相等"。
- **看结果的方式本身也会假通过**：`cargo xtask … | tail -45` 的退出码是 `tail` 的，
  管道把 harness 的失败吞成了 `exit=0`。board 那侧的镜像工具也有同一类陷阱（见上）。
  核对通过与否要直接留退出码（重定向到文件后 `echo "EXIT=$?"`），不要接管道。
- 计数器那一步（net_queue 前后采样）原先写成一条 373 列的单行命令（按每折 65 列算会折成六段），
  正是上面折行规则的反例，它一直在折行失败；按同样的短行规则改写后，四个步骤的判据才都落在
  被断言的路径上。
- 这一轮最终跑出来的结果是可信的：五个区间 `irq_wait 19/18`、`rx_publish 19/19`、
  `rx_residence 20/20`、`tx_residence 22/21`、`tx_device 19/20`（1/4 与 1/16 两个速率），
  全部在一个桶内。
- **不再拿"每帧都打戳"当参考**：那本身是一份真实工作量，属于第三种配置而不是基准。
  判据改成两个轻量速率互比，理由写在配置注释里。
- 对照轮把 helper 改回裸 memcpy 时，**备份是在改动之后做的**，于是"恢复"恢复的是
  被改坏的版本，其后两轮跑的都是对照实现，白跑且误导。教训：改动前先备份，
  恢复后用能区分两个版本的判据核对（只数 `copy_from_kernel_nofault` 的出现次数
  区分不了——两版都有两处）；跑之前确认构建日志里出现了 `Compiling starry-kernel`。

## 待办与前置

已经解决的两条：

- ~~阶段零是硬前置~~：已完成，见周期表与对照轮。
- ~~载体要改可移植驱动接口，波及四个驱动~~：**不成立**。载体放在 `DmaBuffer` 上，
  驱动只移动这个令牌、不认识这个字段，四个驱动一行都没改。改 `ITxQueue::reclaim`
  的签名才会真的波及它们。

还开着的：

1. **R4：关闭态逐帧成本的结构已核对，周期数仍只能在板上量。** 现在的形状是两层：
   逐帧无条件的一层只有"计数器自增 + 采样率读 + 掩码判断 + 一次载体存储"，
   报告本身（构造事件、取观察者指针、进观察者对事件分派、逐事件的 `Acquire` 门控）
   全部落在"这一帧带载体"这个分支之后，未采样的帧不进这一层。
   另外两处每帧判断是执行器提交/回收循环里的 `carrier() != 0`。
   把逐帧报告压到采样之后是设计本意，但发送侧原先在采样判定之前就读了一次时钟
   （`rdtime` 加一次函数调用），未采样的帧也一样付——P11 把它移进了采样分支，
   与接收侧对齐，也让未采样路径上不再有任何函数调用。
   逐事件门控本身不便宜（观察者入口按最坏情况保存 13 个被调用者保存寄存器，关闭态照付），
   但它只落在采样帧与每次轮询一次的 `net:queue_poll` 上，不在逐帧固定成本里。
   QEMU 的 CPU 是模拟的，这些指令在板上值多少周期没有意义，只能上板量。
   注：本分支的构建产物里，观察者指针的非空判断仍在，但调用目标被折叠成直接调用
   （该静态量全程序只有一个常量写入点），启用入口不是间接调用。
2. **探针开销的量化只能在板上做。** QEMU 里能做的是"采样 vs 全采样"的形状对比
   （P8 已做），吞吐与 CPU 的绝对值要上板。
3. **SG2002 的核数与 per-CPU 语义**未确认（影响 per-CPU map 的必要性）。
4. **`wifi:control` 与 `wifi:sdio_xfer` 未做**，QEMU 镜像里没有对应驱动。
5. 6 处 `#[inline(never)]` 还在（撤除排在阶段二之后，且它本身是一次对照实验）。
6. **`net:route_result` 与 `net:queue_backpressure` 只走到"接通"。** 前者的通路访客到不了
   （超 MTU 的判据是 IP 包长 > 1500，IPv4 分片让它不成立；其余 reason 要求执行器组已消失），
   后者要 TX ring 被填满。两者都要故障注入或实板，QEMU 的常规负载给不出来。

## 前身：kprobe 原型阶段（已归档）

`archive/` 中保留的历史，其数据与教训在新方案中仍然有效：

| 文档 | 仍然有效的部分 |
| --- | --- |
| `netmon-plan-kprobe.md` | 构建环境前提（bpf-linker 与 nightly 的 LLVM 版本必须成对升级）；验收口径（吞吐验收在监测关闭时采集） |
| `netmon-tracker-kprobe.md` | 周期 E1–E3：attach 卡点的根因定位与修复（`axmm` 可执行内核区改基页映射）——该修复与路线无关，已保留在分支上 |
| `pr-draft-kprobe.md` | kprobe 版本的 PR 陈述草稿，按新共识需重写 |

从原型阶段得到的、被新方案吸收的结论：release 内联会消灭符号（因此静态 tracepoint 不依赖符号
是优势）；单槽时间戳在并发下会互相覆盖（因此改为随包携带载体）；
"IRQ 到 poll 的次数比"不是一对一（因此 `queue_poll` 按 sequence 区间建模）。

### 两处需要记住的 QEMU/harness 行为（本轮踩到）

1. **用例失败后 harness 不会杀掉 QEMU**。r9 因 `NonHelperCall` 失败后，那个 guest 一直活着；
   我随后跑的 r10 与它并存，r10 的内核在 flows 步骤里抛了
   `memory allocation of 376 bytes failed`（`ARCEOS_PANIC_EMERGENCY`，即内核侧 no_std 的
   `handle_alloc_error`）。r10 之后在干净机器上重跑（r12）五步全过，且同一轮里
   `flow_events` 只有 135 条、eBPF 侧每次事件只分配两个几十字节的 `Vec`（LRU 表按 key 替换、有界），
   本次改动解释不了 512 MB 级别的耗尽。结论：**先清干净残留 QEMU（`ps -eo pid,comm` 里
   `qemu-system*`）再跑下一个用例**，并且别用 `pkill -f`（模式会匹配到自己的命令行，
   实测把自己的 shell 打死，退出码 144）。
2. **busybox `nc` 在这个 rootfs 上不可用**（`apps/starry/nginx/smoke/nginx-smoke-tests.sh`
   早就写了这条：打印 `punt!` 后退出）。`run_load` 原先的两次 `nc` 推送因此既没有流量、
   又各耗掉一次 timeout，已删除；现在的负载就是一次 4 MB 下载。

## 审查第二轮（阶段四）与迭代

第二轮四个 reviewer（principal×2、quality×2）审的是 `dev...HEAD` 全量。结论里**没有 blocker**，
但有三条是我自己没意识到的真问题，都改了：

1. **close 记到了别的 flow 上**（两位 principal 都报）。`report_flow` 每次都用 socket *当下*的
   状态重建 key，而 `shutdown(SHUT_RDWR)` 会把 `bound_endpoint` 清空
   （`tcp.rs:796/815`），于是 Drop 里那条 CLOSE 落在"local 为空"的 key 上，
   真正的 flow 永远停在 `opens=1 closes=0`（读起来像还活着）。connect 失败后被清 peer 的那条
   同理。改法：**key 在 open 时取一次并存进 socket**（`flow_key: SpinLock<Option<FlowKey>>`），
   之后 TX/RX/CLOSE 都用它，不再重建。被拒的 connect 现在会得到 `opens=1 closes=1`。
2. **eBPF 侧那次"读旧值"仍然透过 map 指针**。`kbpf-basic` 的 lookup 返回内部 `Vec` 的裸指针、
   锁已释放，另一个 CPU 的同 key insert 会把旧 buffer 释放掉——原注释只论证了*写*的一侧。
   改成**逐字段走 `bpf_probe_read_kernel` 拷出**（11 次 helper 调用：3 个 record 字段 + 8 个
   entry 字段），读失败即当作"没有旧值"；写回仍然是整体 `insert`。这样最坏情况是这条事件丢基线，
   而不是在已归还的页上出错。单核目标上不会发生。
   顺带核对过：换成 `BPF_MAP_TYPE_LRU_PERCPU_HASH` 并不能解决——kbpf 把它实现成"每个 CPU 一张
   独立的表"，于是 userspace 只能枚举 syscall 所在那个 CPU 的 key，会静默漏掉别的 CPU 的 flow。
3. **`rto_micros` 结构性恒为 0**。smoltcp 的 `Socket::timeout()` 是 `set_timeout()`（用户 abort
   计时器），不是 RTO；`tcp.rs` 里根本没人调 `set_timeout`，`TCP_USER_TIMEOUT` 只存不用。
   于是这一列永远是 0 却标着 RTO。**删掉该字段**，并把"RTO 当 RTT 代理量"的口径从 README、
   plan、observe.rs 的注释里一并去掉（smoltcp 的 RTT/重传计数拿不到，就不假装拿得到）。
   value 因此变成 4×u64 + 4×u32 = 48 字节、无 padding，并新增 `first_ns`（首次事件时刻，
   与 `last_ns` 一起给出 flow 的生命期）。

另外改掉的语义与文档问题：

- **`yielded` 不是饱和信号**（principal-2，我原来的 README/注释写错了）。executor 最多连续 10 次
  轮询或 2 ms 就把 CPU 让给 device owner，而预算只在"让出"时重置，所以**空闲时每 10 次轮询就会
  出现一次**。它是"两个 owner 轮流"的次数（公平性），不是压力；饱和要看 `more`。
  observe.rs / README / eBPF 程序 / loader 的措辞都按这个改了。
- README 的采样分类：`hist_proto_poll_dur` 属于"每个事件都记"那一类，原文"every `hist_*`"是错的；
  flow 计数与 `proto_poll` 也补进了清单。
- `--flows` 的输出行改成 `flows=<读到> unread=<读不到>`，`unknown-key` 带上版本号；
  `print_flows` 的注释里"零字节的流不打印"也是错的（只 open 过的流就是要打印的）。

检查侧的加固（quality-2）：

- `check_flows` 原先取的是**第一次快照**的第一行（可能还在建连），且只断言"非零"——
  一个"每条事件覆盖写"而不是累加的表也能过。现在取**最后一次快照**里最忙的一行，
  并断言 `rx_bytes/tx_bytes ≥ 64 KiB`、调用数 > 1、`opens ≥ 1`。
- `rates` 的速率 0 那一组原先可能**空过**（`median` 对"文件不存在"也返回 -1）：现在同时断言
  `queue_poll > 0` 与 `irq_wait` 中位桶 ≥ 0，即"不打时间戳的那半边也在计数"。
- `check_counters` 的空计数器不会再因 `set -u` 把 shell 打死（那样会绕过新增的失败标记，
  又退回 600 s 超时）；`check_symbols` 也补了标记与失败分支。

### 迭代中被内核 verifier 挡住的第二条约束

修 fault-safe 读那次，程序**加载失败**，但这次不是 `NonHelperCall` 而是
`BackwardBranch { index: 303, target: 283 }` —— 内核的有界性检查是"控制流在指令流里
只能向前推进一次"，而 LLVM 会为了 tail merge 把跳转目标放到跳转之前（反汇编里就是
`goto -0x6`）。逐字段的 `bpf_probe_read_kernel` 读法必然引入多个"取到/取不到"的菱形分支，
每个菱形都是一次分支合并，最后几乎一定出现回边；eBPF 没有条件传送，`unwrap_or` 也只能展开成分支。
试了三种写法（`Option<FlowStats>` 返回值、扁平赋值、分支无关的计数），回边数 3 → 2 → 4，
都不收敛。**结论：在这个内核上，"读一个可能已失效的 map 条目并且逐字段容错"这种形状写不出来。**

因此该处回到"直接读 `FLOWS.get(&key)` 得到的引用"，并把**真正的理由**写进代码与 README：
flow 事件只从 socket 调用点发出（不在中断上下文）、只有这一个程序写这张表、目标是单核——
所以读期间条目不会被换出；多核构建上另一 CPU 可能把它换出，那时读到的是已释放内存，
计数器也会丢更新。这个取舍连同"为什么不能做得更好"一起记在代码注释里，而不是假装安全。

README 的"内核加载约束"一节相应补上第二条：`goto -0x` 与 `call -0x1` 一样会被拒，
`llvm-objdump -d --section=tracepoint <obj>` 可以同时看这两样。

## 本轮板测镜像

`C:\Users\Asta\Desktop\build\board-netmon-flows-20260929.img`（2.58 GB，基准是
`sg2002_starryos_wifi_sta_ph3_20260929.img` 的 rootfs）。出镜像时脚本自己的回读核对全过：
内核里含 SSID、boot.sd 的默认配置与载入地址、FIT 无 ramdisk、`/chosen` 无 initrd、
p2 里的三个初始化文件与两个负载（`/usr/bin/netmon`、`/usr/bin/netmon-board.sh`）逐字节一致。

两个坑记一下：`IMAGE_TOOL` 要用 **opt 工作树里**那一个
（`wt-sg2002-wifi-opt/www/sg2002-wifi-irq/sg2002-image-build`）；`sg2002-image-build` 在
`tgoskits/` 下也确实存在同名目录，但里面没有那套脚本，指错了会以"缺少 …/scripts/…"退出。

## 板测结果（2026-09-30 那一轮，镜像 board-netmon-flows-20260929）

日志：`logs/[com COM6] (2026-09-30_075258) …`。四个组按 off / observe(rate0) / sampled(rate16) / off2 跑。

**成立的：**

- **flow 表在板上是准的**：`flow tcp 192.168.137.235:49157->192.168.137.1:5201
  tx_bytes=6823044 tx_ops=713 rx_bytes=0 rx_ops=0 opens=1 closes=0 first_ns=… last_ns=…`，
  端点对、端口（5201 是 PC 侧 iperf3 服务端口）都对；`flow_events=732 flow_open=2 flow_tx=720
  flow_close=2 flow_insert_failed=0` —— 两次 open 是 iperf3 的控制连接+数据连接，720 次 tx 事件里
  713 次落在最忙那条上，与表里的 `tx_ops` 对得上；`flow_insert_failed=0` 说明表没丢过条目。
  6.82 MB 也对得上 iperf3 报的 5.50 MB/20 s 加上 `-O 3` 预热那段的字节。
  `closes=0` 是因为监测在 socket 被 drop 之前就停了（sampled 组那次 monitor 收到 TERM）。
- **速率 0 的结构性判据在板上成立**：`carrier_rate=0` 时 `hist_rx_publish/hist_tx_device` 全为 0、
  `rx_enqueued/tx_submitted` 全为 0，而未打时间戳的那半边照常计数
  （`queue_poll=15449 queue_irq=18476 irq_wait=17`）—— 与 QEMU 的断言一致。
- **五个区间复现上一轮**：`intervals irq_wait=17 rx_publish=13 rx_residence=22 tx_residence=23
  tx_device=26`（上一轮 17/13/21/23/26）。
- `proto_poll=2701 more=681 yielded=1276`：`more` 约 25%（负载信号），`yielded` 47%（让出 CPU 的比例，
  在这个 CPU 上 2 ms 截止时间经常先到）。

**板测查出来的真问题：**

- **flow 表的枚举会绕圈**。快照里 `flows=` 出现 88 / 896 / 526 / … / 46689 这类数，而表最多 128 项，
  行还是同一条重复。原因是内核按自己的"最近使用"顺序走表并返回"这个 key 的下一个"，
  而 eBPF 程序同时在写表（lookup 会把条目提到最前），于是走一圈又回到读过的 key。
  QEMU 上负载轻、没暴露；板上连接活跃就漏了。已在 loader 侧加护栏：遇到重复 key 即停、
  读到的条目数不超过表容量，并在 `flows=` 行加 `capped=`；QEMU 检查也加了"读到条目数 ≤ 128"
  这一条，用来盯住这个形状。
- **`off2` 对照组失败**：iperf3 53 s 没给出汇总（板端 client 有卡死史）。因此这一轮没有
  "off 的同条件复测"，漂移无法界定——`observe` 1.78 Mbit/s 与 `off` 8.28 Mbit/s 的差距里
  有多少是观测代价、多少是链路漂移，这一轮分不开（上一轮 off 7390 / off2 4300，漂移确实存在）。

### 第二轮板测（board-netmon-flows-r2-20260930）与随之的两处修复

- **flow 表在硬件上完整正确**：`opens=1 closes=1` 出现（close 落在自己那条流上，即审查查出的那个
  问题在板上确认修好）；`flows=2 unread=0 capped=yes` —— 绕圈被记成 `capped`，不再是 46689。
- **对照组这次成了**（`off2`=4400 kbit/s），暴露了同一轮内 1.68 倍（off 7390）的漂移：
  以同条件 `off2` 为基准，`sampled` 低约 2 倍。**结论：没有 `off2` 的轮次不能用来算观测开销。**
- `observe`（速率 0）那组两次都没出汇总，最便宜的那个观测配置仍缺一个数；本轮 5 次 iperf3 加载
  失败 2 次，板端 client 卡死率不低。
- **日志量**：820 行 `slow_event queue_rearm` vs 172 个快照。原限流是"每次 drain 8 行"，而
  `queue_rearm` 每轮约 9000 次（快照计数 8707…9541），于是每秒刷 8 行。改成**每种事件整轮最多 8 行**
  + 结束时一行 `slow_events_suppressed`；事件仍从 ring buffer 读走，计数不受影响。
- 另记一条观测（不属本分支目标）：`queue_rearm` 注释里当"罕见"，实际每 20 秒几千次且 `detail=1`
  （确有活等待）——队列那条线可以看看。
