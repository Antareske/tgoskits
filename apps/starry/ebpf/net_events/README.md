# net-events

`net-events` 用 eBPF 挂载六个 `net:*` tracepoint，并按需打印它们的读数。**工具本身不产生任何流量**：装载一次，计数在内核里累加，你在同一个 shell 里手动跑 iperf3，读数的时机由你决定。

## 用法

```sh
net-events &          # 装载六个事件，打印第一份读数，提示符立刻回来
iperf3 -c <对端>      # 你自己敲的命令
kill -USR1 %1         # 打印下一份读数：累计值 + 相对上一份的窗口增量
kill %1               # 分离退出（退出前再打一份读数）
```

屏幕上从头到尾只有读数：装载后打印 `[1]`，之后每次触发打印一份同样形状的读数，没有启动横幅、没有单独的基线块。

前台运行时回车也可触发；板卡场景下 PC 侧打流量、板子控制台按回车出读数即可。

```sh
net-events --events queue_poll_round,tx_submit   # 只挂部分事件
net-events --help
```

## 输出

客户机 x86_64 QEMU（`-smp 4`）的一次运行，流量是客户机里手敲的一轮 `iperf3 -c 10.0.2.2 -t 3`（宿主对端跑 `iperf3 -s`）：

```text
[1]
  queue_poll_round             10          +10   idle=10 more=0 blocked=0 failed=0   work_units=540 (54.0/round)
  queue_rearm                   4           +4   race=4 work_pending=0 retry_at=0 failed=0
  queue_backpressure            0           +0   not driven in this window (needs a busy device)
  tx_submit                   134         +134   frames=134 bytes=204390 (avg 1525 B/frame)
  rx_publish                  114         +114   frames=114 bytes=6210 (avg 54 B/frame)
  proto_yield                   4           +4   poll_count=0 deadline=4 both=0   work_pending=3

[2] +3.1s since [1]
  queue_poll_round           1353        +1343   idle=1343 more=0 blocked=0 failed=0   work_units=127095 (94.6/round)
  queue_rearm                 698         +694   race=692 work_pending=2 retry_at=0 failed=0
  queue_backpressure            0           +0   not driven in this window (needs a busy device)
  tx_submit                 25466       +25332   frames=25332 bytes=38253143 (avg 1510 B/frame)
  rx_publish                25470       +25356   frames=25356 bytes=1369501 (avg 54 B/frame)
  proto_yield                 709         +705   poll_count=0 deadline=705 both=0   work_pending=640

[3] +1.0s since [2]
  queue_poll_round           1353           +0   not seen in this window
  queue_rearm                 698           +0   not driven in this window (needs a race at rearm)
  queue_backpressure            0           +0   not driven in this window (needs a busy device)
  tx_submit                 25466           +0   not seen in this window
  rx_publish                25470           +0   not seen in this window
  proto_yield                 709           +0   not seen in this window
```

这一屏把 iperf3 的流量形状说清楚了：发送侧 23496 帧、平均 1510 字节（满载数据帧），接收侧 23519 帧、平均 54 字节（14 以太网 + 20 IP + 20 TCP 的纯 ACK），队列每轮做 100 个工作单位、618 次 rearm 全部是竞态结局，协议执行器让出 643 次、其中 602 次仍有工作。

和接口侧对得上：宿主 `iperf3 -s` 报收到 34.0 MB，而事件里的发送字节是 35.5 MB，差额是逐帧的以太网/IP/TCP 头部。


## 状态

已合入 dev 的六个事件：`queue_poll_round`、`queue_rearm`、`queue_backpressure`、`tx_submit`、`rx_publish`、`proto_yield`。字段顺序与结果码取值以 `os/StarryOS/kernel/src/tracepoint/net.rs` 与 `net/ax-net/src/observe.rs` 为准；工具在附着前用 `format` 校验一遍顺序。

## 设计要点

- **挂载即开闸**：perf 事件附着本身就是打开该事件 gate 的动作，不需要写 tracefs 的 `enable`。
- **输出只有读数**：装载后立刻打印第一份读数，之后每次触发打印一份同样形状的读数；没有启动横幅、没有基线块，屏幕上从头到尾都是同一形态的读数。
- **附着前校验字段顺序**：读取每个事件的 `format`，确认它声明的字段与 eBPF 程序按偏移读取的顺序一致；不一致就拒绝挂载。字段挪位而偏移照读，会把邻居的字节当成合法读数。
- **每个读数是窗口口径**：内核里是累计计数，打印时减去上一份。帧长记「帧数 + 字节数」，窗口平均帧长由增量算——全局 min/max 描述的是「自挂载以来」，跨窗口会误导。
- **计数按 owner_cpu 分区**：见下节。

## 为什么不是 per-CPU map

本内核的 per-CPU map 从用户态读回的是**执行这次系统调用的那个 CPU 的那一片**，而不是 Linux 的「所有 CPU 的值」：

- `kbpf-basic` 的 `PerCpuArrayMap::lookup_elem` 走 `per_cpu_data.get_mut().lookup_elem(key)`；
- StarryOS 的 `PerCpuVariantsImpl::get_mut` 按 `this_cpu_id()` 取那一个 slot（`os/StarryOS/kernel/src/ebpf/transform.rs`）。

所以「用户态把读回的 per-CPU 值求和」得到的是读者所在 CPU 的计数，且随读者被调度到哪个 CPU 而漂移。同一内核也没有原子 map 指令，无法用「共享计数器 + 原子自增」。

因此计数表是**普通 `Array`，按事件自带的 `owner_cpu` 分区**：每个 CPU 只写自己那一片（队列执行器按设计固定在归属 CPU 上报告），读改写因此无竞争；普通 Array 的读取与 CPU 无关，把所有分区的同一槽位求和即得全局读数。即使某次报告发生在非归属 CPU 上，增量仍落在某个合法分区，跨分区求和依然正确。

分区数固定在 `PARTITIONS`（8），超出时按 `%` 折叠共享分区而不是丢弃事件。

## 边界

- 事件可以被关闭或丢弃，一个读数只统计事件报出来的事实，不代替 `/sys/kernel/debug/net_queue` 与 `/proc/net/dev` 的累计。
- `queue_backpressure` 需要设备真的忙，`queue_rearm` 的非空闲结局需要真的发生竞态；未驱动时工具按「本窗口未驱动」打印，不判失败。
- 本工具是操作者演示，不是测试：语义断言仍由 `test-suit/starryos/qemu/system/net-queue` 与 `net-events` 承担。

## 构建与运行

```sh
cargo xtask starry app qemu -t ebpf/net_events --arch x86_64
```

`prebuild.sh` 交叉编译静态 musl 二进制并装入 rootfs overlay。

### QEMU 里用 iperf3 驱动

板卡镜像自带 iperf3，QEMU 的 Alpine 镜像不带，需要注入。注入走仓库既有的 overlay → `debugfs` 链路：`prebuild.sh` 读取 `NET_EVENTS_IPERF3_DIR`，把该目录下的 `iperf3` 与 `libiperf.so.0` 装进 overlay，再由框架注入 rootfs。

取这两个文件的办法（客户机是 musl 的 Alpine，不能直接用宿主 glibc 的 iperf3）：取对应发行版与架构的 iperf3 包，解出 `usr/bin/iperf3` 与 `usr/lib/libiperf.so.0.0.0`（后者按 SONAME 命名为 `libiperf.so.0`）。客户机已自带 `libcrypto.so.3` 与 musl 运行库，因此不需要再注入别的东西。

```sh
# 宿主：起服务端
iperf3 -s

# 构建并启动客户机（注入 iperf3）
NET_EVENTS_IPERF3_DIR=<放 iperf3 与 libiperf.so.0 的目录> \
    cargo xtask starry app qemu -t ebpf/net_events --arch x86_64

# 客户机内：装载、跑一轮、取读数
net-events &
iperf3 -c 10.0.2.2 -t 3
kill -USR1 %1
kill %1
```

客户机经 QEMU 用户态网关访问宿主，`10.0.2.2` 即宿主的 loopback，因此宿主上的 `iperf3 -s` 默认监听配置就能被连上，不需要额外的端口转发（那是宿主主动连客户机才需要的方向）。
