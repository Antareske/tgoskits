结论：请求修改。L2 长度计算本身方向正确，但当前实现仍有两个会让 `/proc/net/dev` 统计偏离 Linux 语义或在背压下错配的阻塞问题，详见行内意见。

核查结果：
- 正确性/兼容性：Ethernet header、ETH_ZLEN padding 与排除 FCS 的长度口径合理；但成功收发的 ARP 帧被排除，不能称为 Linux netdev 语义。
- 并发/队列：RX 共享队列满时，临时 `frame_lens` 与持久 `rx_buffer` 的生命周期不一致，会把下一轮长度配给上一轮包。
- 安全性、unsafe、硬件 ABI：本补丁未引入新的 unsafe；`NetRxBuffer` 的 FCS 合约与当前唯一 `RdNetDriver` 实现一致。
- 测试：新增测试位于 `ax-net` 单元测试层，但仅验证 mock 返回值和原子累加，没有执行 `EthernetDevice` 的 ARP/短帧路径，也没有覆盖 RX queue backpressure，因此未捕获上述问题。请补充确定性回归。
- 本地验证：`cargo fmt --all -- --check` 通过；`cargo test -p ax-net` 通过（52 tests）；`cargo xtask clippy --package ax-net` 通过（base、vsock）。
- CI：当前 head 的 fmt、clippy、std tests、Starry 四架构 QEMU、ArceOS/Axvisor 及列出的 self-hosted board jobs 均通过。PR 正文中的 `cargo starry test ...` 命令不是当前推荐的 `cargo xtask` 入口，但 CI 提供了实际构建/运行证据。
- 重叠检查：#1417 是用户态 net-bench/eBPF 观测，功能互补；#1566 同时修改 `Device` trait 与 router 以加入 TUN/TAP，目标不重复，但合并时需要 rebase 并适配新的返回值合约。未发现重复实现。
- 前序讨论：3 条 rustfmt 意见已由 `1729ba3` 修复并已标记 resolved。



net/ax-net/src/device/ethernet.rs:335-337

                frame_len
            }
            EthernetProtocol::Arp => {


[P1] 请把成功收发的 ARP 帧纳入 netdev 统计。这里处理 ARP 后返回 0，RX worker 因而不会增加 rx_packets/rx_bytes；同文件的 request_arp() 与 ARP reply 路径又明确丢弃 send_to() 返回的长度，所以 tx_packets/tx_bytes 也会遗漏这些已成功发送的帧。Linux networking statistics 规定 packets 统计所有 good packets，IEEE 802.3 的 bytes 统计不含 FCS 的 Ethernet frame；控制帧不能仅因未上送 IP 层就从接口统计中消失。请拆分“是否向 IP buffer 入队”和“成功收/发 L2 帧的统计事件”，并增加真实 EthernetDevice 的 ARP request/reply 回归测试。参考：https://www.kernel.org/doc/html/latest/networking/statistics.html


net/ax-net/src/router.rs:1013-1014

                ax_task::yield_now();
                break;

[P1] 这里提前 break 会让帧长与包永久失去对应关系：frame_lens 只保存本轮 recv() 的长度，但 push 失败后 rx_buffer 中其余旧包会留到下一轮；下一轮会用新收到帧的长度去统计先出队的旧包，没有新帧时旧包还会继续滞留。请让长度与包存放在同一个持久 FIFO/metadata 中，或跨轮保留尚未消费的长度，并增加一个先填满 rx_queue、触发失败后再恢复消费的确定性测试。