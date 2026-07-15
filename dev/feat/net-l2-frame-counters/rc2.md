本轮复审确认 ARP request/reply 的实际 EthernetDevice 路径现已记录成功 RX/TX 帧长，生产 worker 也改为持久 FIFO 配对，修复方向正确；但仍有两处 Linux netdev 统计遗漏和一项不锁定生产代码的回归测试阻塞。

聚焦验证：
- cargo test -p ax-net --lib arp_counter_tests -- --nocapture：7 passed。
- cargo test -p ax-net --lib l2_counter_tests -- --nocapture：12 passed。
- cargo test -p ax-net --lib：62 passed。
- cargo xtask axvisor test qemu --arch aarch64：在 current head 成功启动 Linux guest，输出 guest test pass!，1/1 通过，退出 0，总耗时 106.40 秒。
- git diff --check origin/dev...HEAD：通过。

current-head CI 为 success=28、skipped=29、cancelled=1。skipped 是预期互斥矩阵；唯一取消项 Test axvisor aarch64 qemu / run_host 在 early core-0 virtualization init 后无输出约 6 小时。exact base 同名 job 成功，改动面仅 ax-net，且上述 current-head 本地精确命令成功，因此按一次性 runner/QEMU 挂起分类，并已创建跟踪 issue #1598：https://github.com/rcore-os/tgoskits/issues/1598 。其余格式、clippy、std 与正常矩阵由 current-head CI 覆盖，未重复宽泛检查。

Linux rtnl_link_stats64 语义检查显示：成功交给设备的 TX 事件不能因 IP 重配置被撤销；rx_packets 应包含从设备收到的所有 good packets，即使之后因不支持协议而丢弃。当前 set_ipv4_addr 会清除尚未 drain 的已成功 ARP TX 统计，且非 ARP 的有效 L2 帧仍在 _ 分支返回 0 而不计数。背压测试还复制待测循环，无法在生产 worker 回退到旧错误时失败。

重复/重叠：origin/dev 无等价实现；#1566 非重复但共同修改 ethernet/device/router 合约，属于 conflict-risk，若本 PR修复后建议先合入 #1571，再让 #1566 rebase 适配 usize/deferred-drain API；#1417 是 complementary app-level net_stats 使用，#1574 与此无关。无 Cargo/lock/[patch.crates-io] 或新增 unsafe。PR 正文仍写旧的 drain_async_tx/9 tests，与当前 drain_deferred_tx/rx 和 19 个 focused tests 不一致，修复后请同步描述。

现有 ARP 专项线程已由当前生产路径与真实 EthernetDevice 测试满足，将解析；backpressure 线程虽然 outdated，但测试仍未执行生产 worker，保持 open。



net/ax-net/src/device/ethernet.rs:667-670

【P1｜阻塞】这里会丢失已经成功交给设备的统计事件。可复现时序是：TX worker 对未知邻居调用 send()，ARP request 已由 transmit() 成功并把 60 记入 deferred_tx_frame_lens；它释放锁后，运行时 IPv4 配置更新经 Router::ipv4_rules 调用 set_ipv4_addr()；RX worker 尚未 drain 时本行 clear，最终 tx_packets/tx_bytes 永久少 1/60。Linux rtnl_link_stats64 的 tx_packets 定义是成功交给设备的包，且统计应跨日常接口操作保留；IP 上下文变化不能撤销已经发生的链路事件。请在重配置前原子地 drain 并计数，或让发送路径直接发布计数而不由另一 worker 延后；补充上述确定性交错回归。


net/ax-net/src/device/ethernet.rs:348-351

【P1｜阻塞】当前 side channel 只记录 ARP；同一 match 的 _ => 0 会让已通过 Ethernet 解析和目的 MAC 过滤的其他有效 L2 帧，例如不受协议栈支持的 EtherType/控制帧，被回收却不增加 rx_packets/rx_bytes。Linux 明确定义 rx_packets 包含主机从设备收到的所有 good packets，即使之后因不支持协议等原因丢弃；rx_bytes 与这些包对应。请在完成 L2 有效性/本机地址检查后把所有未上送 IP buffer 的 good frame 都记录到 deferred RX，ARP 再额外执行 process_arp，并加一个非 ARP EtherType 回归。


net/ax-net/src/router.rs:1724-1727

【P1｜阻塞】这段测试重新实现了待验证的背压循环，而没有执行生产代码中的 device_rx_worker 或可单测的 worker step。因此即使把生产实现恢复为旧的 frame_lens 临时数组/提前 break 错配逻辑，这个测试仍会照样通过；本地 cargo test 只能证明测试里这份副本正确，不能作为该 bug 的红绿回归。请把单轮 recv/配对/推送/回退提取为生产辅助函数并由 worker 与测试共同调用，或用可终止的 worker harness 驱动真实路径，验证先填满共享 RX 队列、接收不同长度帧、恢复消费后统计仍为 100/200/300。