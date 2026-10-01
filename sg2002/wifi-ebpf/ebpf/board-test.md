# SG2002 上板测试（网络观测线）

本文件是走一遍上板的操作说明与判据。板侧脚本是 `netmon-board.sh`（同目录）。

## 这次上板要回答什么

方案里明确过：**吞吐验收必须在监测关闭时采集**（历史教训是探针把吞吐拉低了 19%），
延迟分布可以在监测开启时采集，但要同时记录采样率与开关状态。所以板侧跑三组：

| 组 | 配置 | 回答什么 |
| --- | --- | --- |
| `off` | 不跑 netmon | **吞吐的验收数字** |
| `observe` | netmon 挂着，采样率 0（没有帧带载体） | 观测路径自身的固定成本 |
| `sampled` | netmon 挂着，采样率 16 | 五个区间的分布在真实设备上的形状；采样开销 |
| `off2` | 再测一次 `off`，放在最后 | **对照**：见下 |

`off2` 是因为这块板子的吞吐会自己往下掉（实测一轮里 `off` 7.7 → `observe` 2.0 Mbit/s）。
只有 `off2` 回到 `off` 的量级，"`observe`/`sampled` 比 `off` 低"才能读成观测成本；
若 `off2` 也低，那就是漂移（散热/AP/驱动），与观测线无关。

## 前置：网络与镜像

### 板侧联网

WiFi 用 sta 模式，SSID `aasta`、密码 `12345678`。**关联是编译期做完的**，由 aic8800 驱动自己
发起（见下面第四条留意），所以板子上不需要 wpa_supplicant 之类的东西，也不需要敲联网命令。

两个前置因此必须在上电前就位：

- **那个 AP 要在场**。关联发生在启动时，AP 不在场就没有地址（凭据取错时更要紧：关联被拒会 panic）。
- **DTB 里的 `rng-seed` 不能缺**（WPA2 握手要启动熵；脚本负责补）。

### 镜像怎么出

基准镜像用 wifi-opt 线最新的整盘镜像（`C:\Users\Asta\Desktop\build` 下
`sg2002_starryos_wifi_sta_attrib3_20260929.img` 一类），**借的是它的 rootfs**：
init 资产与包都在 p2，而内核与 DTB 在 p1 的 `boot.sd` 里，换内核不影响 rootfs。
本分支是 dev + 观测线，WiFi 走 dev 的那一套。

镜像工具在另一个工作树里（`wt-sg2002-wifi-opt/www/sg2002-wifi-irq/sg2002-image-build/`）。
出镜像用本目录的 `build-board-image.sh`：它编译（或复用）带凭据的内核、派生 DTB、重打 FIT、
注入 p2，并把每一步的结果回读比对（FIT 组件、DTB 的 `/chosen`、p2 里的文件、内核里有没有
SSID），有任何一项不符就报错退出。

```
BASE_IMAGE=<基准镜像.img> OUT_IMAGE=/mnt/c/Users/Asta/Desktop/build/<新镜像>.img \
    IMAGE_TOOL=<工具目录> \
    STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 \
    sh www/ebpf/build-board-image.sh
```

`STARRY_WIFI_SSID` / `STARRY_WIFI_PASSWORD` 是必给的（缺了脚本直接拒绝），
`AIC8800_FIRMWARE_DIR=<固件缓存>` 可选（省掉构建期联网取固件）；
固件已经在 OUT_DIR 里时也可以 `SKIP_KERNEL=1` 复用它，但脚本仍会核对凭据。

脚本在本树根目录下调用工具，所以 `--work` 落在本树的 `.sg2002-build/`，不会动那个工作树的资产。
p2 的起始字节偏移是 `68157440`（扇区 133120 × 512），注入与校验都用它。

六点要留意，前四条是把这块板子跑起来过程中踩到的：

- **DTB 与 FIT 都不能声明 initrd**：`os/StarryOS/configs/board/licheerv-nano-sg2002.dtb` 的
  `/chosen` 里写死了 `linux,initrd-start = <0x87e81000>` / `linux,initrd-end = <0x880ea1a0>`
  （官方 Linux 那套流程里那个地址恒定放 ramdisk）。dev 的 `fb3edd5cf`（#2528）之后，
  内核把这段区间当 host initramfs 严格校验，于是启动就 panic：那个地址上放的是官方镜像的
  Cvitek ramdisk 时判 `Corrupt("NUL in symlink target")`，不放东西时判 `UnsupportedCompression`。
  所以派生件 `licheerv-nano-sg2002-noinitrd.dtb` 用 `fdtput -d` 删掉了这两个属性
  （反编译对比过，只少这两行），FIT 那边也去掉了 ramdisk，使那段地址不再是任何东西的落点。
  去掉之后内核收不到 initrd 区间，走 SD 卡 rootfs（`root=/dev/mmcblk0p2`）。
- **DTB 要有 32 字节 `rng-seed`**：内核把 `/chosen/rng-seed` 当启动熵
  （`platforms/someboot/src/entropy.rs`），且要求恰好 32 字节。WPA2 握手要用它，
  dev 那份 DTB 没有这一项。派生件每次用新取的随机数补上 8 个字。
- **rootfs 的 init 必须取自 dev 自己的 rootfs**：本板 p2 沿自另一条镜像链，里面是原版 Alpine
  的 inittab，`tty1..tty6` 六条 `respawn:/sbin/getty` 会无限重启——StarryOS 的 devfs 只有
  `/dev/ttyS<N>` 与 `/dev/tty`，没有虚拟控制台，所以这六条永远起不来，控制台永远拿不到 shell。
  dev 自己的 rootfs（`target/axbuild/rootfs/rootfs-riscv64-alpine.img`）用
  `::respawn:-/usr/libexec/starry/console` 取代了它们，那个助手 source
  `/etc/profile.d/starry.sh`（其中定义了 `PS1`）后起一个登录 shell。脚本把这三个文件
  从 dev 的 rootfs 取出并注入 p2。
  背景：`fb3edd5cf` 同时把 `legacy-board-init` 从三个 SG2002 配置里删掉了，而在此之前是内核
  自己起 console shell、绕过 rootfs 的 init，所以这份 inittab 的毛病以前不会显出来。
- **STA 凭据必须编进内核**：`drivers/ax-driver/build.rs` 读 `STARRY_WIFI_SSID` /
  `STARRY_WIFI_PASSWORD`，`src/net/aic8800/startup_config.rs` 用 `option_env!` 把 SSID 与
  PBKDF2 算出的 WPA2 PMK 编进驱动，关联由驱动自己发起，rootfs 里不需要 wpa_supplicant
  （官方 Linux 镜像那套 `S30wifi` + `wpa_supplicant -B -i wlan0` 是另一套用户态方案）。
  缺凭据的症状是内核照常起、`wlan0` 也 `UP`，但永远不关联、拿不到地址 —— 脚本因此在编译
  之后、组装之前核对内核里确实含这个 SSID，不合就拒绝出镜像。
  固件（`drivers/net/aic8800/build.rs` 从固定提交 provision 进 OUT_DIR）与启动熵
  （DTB 的 `rng-seed`）两者一并是前提；上游的 `boot_entropy::prepare_for_secure_wifi`
  只接在 `starry run` 板卡运行路径上，裸构建要自己给这两半。
- `update-kernel` 只换内核，**参数表里的 `--inject` 它不读**，写了也不报错、直接出一个没有
  netmon 的镜像。注入必须单独调 `inject-rootfs.sh`（脚本里就是这么做的）。
- **不带 `--dtb` 会顺带换掉 DTB**：`update-kernel` 用的是 work 目录里当前的
  `assets/licheerv-nano-sg2002.dtb`，而那个文件是上一次构建留下的，未必是基准镜像实际
  烧进去的那份。显式指定 `--dtb`，让内核与 DTB 都来自本分支。
- **板级内核与 QEMU 内核共用同一个产物路径**：`cargo xtask starry app qemu` 会重新构建
  `target/riscv64gc-unknown-none-elf/release/starryos.bin`，把先前构建的板级内核覆盖掉
  （配置不同，产物不同）。所以"构建板级内核 → 出镜像"这一步中间不要再跑 QEMU 应用入口；
  要核对镜像里的内核是否为板级那份，用 FIT 自己记录的 crc32 比对，不要拿工作树里的文件比
  ——那个文件可能已经被覆盖。

## 上板之前：先在 QEMU 跑一遍

烧写与插拔都是消耗，所以先在 QEMU 里确认应用侧行为：

```
cargo xtask starry app qemu -t ebpf/netmon --arch riscv64
```

四个检查步骤全过才上板：`netmon-check symbols` 的符号行、`netmon-check rates` 打出的
`TRACEPOINTS_LIVE`、`--selftest` 的 `SELFTEST_END`、`netmon-check counters` 的
`QUEUE_COUNTERS_ADVANCED`。它验的是附着、记录偏移与"五个区间在两种速率下一致"，
与负载是 iperf3 还是下载无关。

QEMU 客户机里没有 iperf3，那一步的负载默认走 harness 的 HTTP 下载。要让它也走 iperf3：
把 `NETMON_IPERF3_DIR` 指向放着 `iperf3` 与 `libiperf.so.0` 的目录（本目录的
`iperf3-riscv64/` 就是从板级镜像里取出的这两个文件），并**在跑之前**让宿主侧的
`iperf3 -s` 起着 —— harness 不会替你起它。日志里会打印那一轮用的是哪种负载。

跑完这一遍，`target/riscv64gc-unknown-none-elf/release/starryos.bin` 会变成 QEMU 配置的那份，
所以**要重出板级镜像时必须先重编板级内核**（见下面第四条留意）。

## 板侧步骤

### PC 侧：起 iperf3 server

板子作 client 连它，所以 PC 侧就是：

```
cd C:\Users\Asta\tools\iperf3
.\iperf3.exe -s
```

默认绑 `0.0.0.0:5201`，热点那侧即 `192.168.137.1:5201`。**不要加 `-1`**：三组配置各开一条连接，
服务完一条就退会漏掉后两组；三组跑完（看到 `NETMON_BOARD_END`）再 Ctrl-C。

管理员 PowerShell 里放行入站 5201 —— 被防火墙挡住的症状是板侧 iperf3 卡到最后超时：

```
New-NetFirewallRule -DisplayName "iperf3" -Direction Inbound -Protocol TCP -LocalPort 5201 -Action Allow
```

也可以用本目录的 `netmon-peer.ps1 -Ip 192.168.137.1`：它会查/加那条规则、把服务端输出落到
`test\iperf3-<时间戳>.log`，并打印下面板侧该敲的三行。手敲一样可行，脚本只是省事。

### 板侧：跑四组

串口控制台（日志照旧落在 `C:\Users\Asta\Desktop\logs`）。**每一行都要短**：
串口 80 列、提示符占 15，超出的部分会被折行、折出来的半行当成一条命令执行。

```
P=192.168.137.1
export NETMON_IPERF_PEER="$P"
sh /usr/bin/netmon-board.sh
```

`P` 填 PC 在 `aasta` 网里的地址。脚本每组跑一次 `iperf3 -c $P -t 20 -O 3`（**TCP、单流**：
板端 UDP 与 `-P4` 有卡死记录，不用），数字取自 iperf3 自己的 `sender`/`receiver` 汇总行，
并把那两行原样打到控制台当证据。板端 client 既然有卡死史，脚本就把 client 放后台、
按时长设死限，超时强杀并把当时的输出打出来，不会把串口堵死。

`NETMON_IPERF_SECONDS` / `NETMON_IPERF_WARMUP` 可改时长与预热（默认 20 / 3，三组一致才可比）。
若确实要用"流到 stdout 的下载器"，把 `NETMON_LOAD` 设成那条命令即可（`NETMON_IPERF_PEER` 优先）；
那种模式下脚本靠 `wc -c` 数字节，一个字都没传回来就直接失败。

`netmon-board.sh` 已经跟着镜像注入到 `/usr/bin/netmon-board.sh`，它会自己跑
`off`/`observe`/`sampled`/`off2` 四组并汇总（`off2` 是 `off` 的同条件复测，用来判断漂移），
不需要分次调用。`sampled` 组另外带 `--flows 8`，所以那一段里还会有 flow 表。**不要粘贴脚本内容**，也不要写成一行——串口折行会把半行当命令执行。

**先确认 `wlan0` 拿到地址**（`ip -4 addr show dev wlan0`），否则流量走不到设备路径上，
测出来的区间没有意义。

## 要回传的东西

脚本最后会打印 `NETMON_BOARD_END`，之前的内容全都要：

- `off` / `observe` / `sampled` 三行的 `bytes/seconds/kbit_per_s`
- 每组 iperf3 的 `sender` / `receiver` 两行（脚本原样打到控制台）——上面那三个数就是从它们算出来的，
  对账要用；PC 侧 `iperf3 -s` 的输出（或 `netmon-peer.ps1` 的日志）里也有同样的三条连接汇总
- `/sys/kernel/debug/net_queue` 的全文（队列计数器，事实来源）
- `observe.mon` 与 `sampled.mon` 两段 netmon 输出：计数、每个直方图的 `count/sum_ns/mean_ns`、
  `intervals` 行的五个中位桶
- `sampled.mon` 里的 `flows=` 行与 `flow ...` 行（flow 表；脚本另打一行
  `netmon-board: busiest flow: ...` 与 `busiest tx_bytes=... rx_bytes=... events=...`）

## 判据

- **吞吐**：只认 `off` 那一行；`off2` 是它同条件的复测，用来判断漂移（见上）。
  `observe` 与 `sampled` 是开销对照，不是验收数字。
- **对端速率**：每组若有 `peer_kbit_per_s` 一行，那是服务端自己数出来的接收速率。
  它与本端 `kbit_per_s` 差得多，说明链路有丢包，不是本端算错。
- **一组失败不等于整场失败**：脚本会把失败的那组记成 `bytes=0`，其余照跑，末尾仍打印全部内容，
  并以 `NETMON_BOARD_GROUPS_FAILED: <组名…>` 标注、退出码非 0。所以即便某组垮了，
  其他组的数字与直方图照样能读。
- **五个区间**：`sampled` 那一轮里 `intervals` 五个中位桶都要 ≥ 0（`-1` 表示那个直方图是空的，
  说明那一档根本没采到样本，要先看 `count`）。
- **计数器自洽**：`tx_submitted` 与 `tx_completed` 应当接近，不要求一个方向的严格不等式。
  背压下前者会大于后者（一次投递算一次，重传会再算一次）；**后者偶尔大于前者是正常的**——
  计数器在 eBPF map 里、随 netmon 进程从零开始，而帧上的标记在内核侧，
  于是在监测启动前就已投递、启动后才完成的帧只会被完成侧计到一次。
  两个会话之间切换、或测量中途重启 netmon，都会出现这种差一。
- **零值**：`hist_*` 的 `count` 是实际采样数。若某个区间的 `count` 明显小于对应的流量计数，
  先查采样率而不是先怀疑设备。
- **flow 表**：`sampled` 那一轮的负载是一条 iperf3 连接，表里就该有它，
  且 `busiest tx_bytes` / `rx_bytes` 至少一边是 MB 量级（脚本把下限设成 64 KiB：
  低于它只说明事件没进表，不说明流量小）。`flow_events` 计数与表里的字节数要能对上量级；
  地址那一列应当就是板端 `wlan0` 的地址与 PC 的地址——它解码的是事件里的端点对，
  对不上说明 key 的布局与解码有出入。

## 这一轮测不到的东西

- **关闭态开销的周期数**：`off` 组能给吞吐差，但"每帧多花多少指令"要另做 cycle 级测量。
- **WiFi 与 SDIO 层**：这两层已从本分支移除（别处在做 WiFi 重构），
  本轮只看网络栈；`wifi0`/SDIO 的 kprobe 不再挂在监测里。
- **`route_result` / `queue_backpressure`**：两个事件在 QEMU 里触发不了，上板能不能触发取决于
  是否出现真实丢包与真实背压；如果没有，它们仍然是"接通但未触发"。

## 自动化的现状

`cargo xtask starry app board` 是**控制台驱动**入口：它把用例的 `init.sh` 送到串口 shell，
按 `board-*.toml` 的提示符与判据判定成败。它不负责把程序放上板子——`apps/starry/README.md`
写明"用例下的用户程序只是示例，除非用例另有说明，否则板级 rootfs 必须已经包含程序及其
共享库"。所以板级的构建与注入由人来做，就是上面两步；`app qemu` 才是 CI 形态的入口
（应用用 `prebuild.sh` 自持构建，harness 用 debugfs 把产物注入 QEMU rootfs）。
本机没有配置远程板卡服务（`cargo xtask board config` 需要交互式终端，`~/.ostool` 不存在），
所以这一轮手工跑控制台，脚本只负责测量与自述。

## 本轮与上一轮的差别（阶段四）

镜像与流程不变，多两件事：

- 监测侧多了 **flow 表**：`sampled` 组带 `--flows 8`，脚本另外断言表里最忙的一条
  至少 64 KiB（一条 iperf3 连接的量级），并把那一行原样打印出来。表里应有
  `tx_bytes`/`rx_bytes`、`opens`/`closes`（iperf3 正常收尾时两边相等）、
  `first_ns`/`last_ns` 给出的生命期。
- `proto_poll` 一行给出 protocol executor 的轮询数与 `more`。
  注意 `yielded` **不是**饱和信号：它数的是"executor 把 CPU 让给 device owner"的次数，
  空闲时每 10 次轮询也会出现一次；看饱和要看 `more` 的比例。

flow 表里**不再有 RTO 列**：smoltcp 的 `Socket::timeout()` 是用户 abort 计时器而不是 RTO，
这个栈里也没有人设置它，所以那一列永远是 0，已删除而不是留着当装饰。
