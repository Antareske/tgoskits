当前 head `46bfef6a88cf27869f23abd5d906116c5e63648a` 复审后仍需修改。

阻塞项：`apps/starry/ebpf/net_stats` 的字节统计没有在实际自测中工作。虽然 QEMU success regex 能匹配到 `NET_STATS_END`，但本地运行 `cargo xtask starry app qemu --test-case ebpf/net_stats --arch x86_64` 后输出为：

```text
tcp_tx_pkts=10  tcp_tx_bytes=0
tcp_rx_pkts=12  tcp_rx_bytes=0
udp_tx_pkts=6  udp_tx_bytes=0
udp_rx_pkts=10  udp_rx_bytes=0
```

这说明 kprobe entry 已经命中，测试也确实产生了 TCP/UDP 流量，但 kretprobe 的 byte 读取没有得到有效结果。当前 `success_regex = ["NET_STATS_END"]` 又没有校验非零 byte counter，因此会把这个核心功能失效误判为通过。请先修正真实 ABI 下的返回值/字节数解析，并让 `--test` 或 QEMU success 条件能稳定捕获 byte counter 全 0 这类回归。

已复核通过的部分：
- `git diff --check origin/dev...HEAD` 通过。
- 变更内 shell 脚本 `bash -n` 全部通过。
- `python3 -m py_compile apps/starry/net-bench/core/*.py` 通过。
- 30 个 net-bench/net_stats TOML 配置解析通过。
- `cargo fmt --check` 通过。
- 未发现 `[patch.crates-io]`。
- 现有 CI 中普通 host/container 项基本通过；OrangePi-5-Plus Starry board job 是等待板卡上电阶段 6 小时超时取消，未看到进入本 PR net-bench/eBPF 逻辑。

补充：本机没有 `iperf3`，所以没有实际跑 net-bench 的 guest/host 吞吐 smoke；但 eBPF byte counter 失效已经足够阻塞当前 PR。



apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs:89-92

这里的 sret pointer 假设还没有在实际运行中成立。当前 head 上我跑了 `cargo xtask starry app qemu --test-case ebpf/net_stats --arch x86_64`，`--test` 会产生 TCP/UDP loopback 流量，但输出是 `tcp_tx_pkts=10/tcp_rx_pkts=12/udp_tx_pkts=6/udp_rx_pkts=10`，同时四个 byte counter 全部是 `0`。这说明 entry probe 已经命中，retprobe 的字节解析没有得到有效返回值；当前实现仍然无法提供 PR 描述里的 TCP/UDP send/recv 字节统计。这里需要按真实 ABI 修正返回值读取方式，并用自测证明 byte counter 为非零。



apps/starry/ebpf/net_stats/net_stats/src/main.rs:191-194

`--test` 现在只打印统计值，不校验统计值是否符合自测流量，所以 QEMU 的 `success_regex = ["NET_STATS_END"]` 会放过 byte counter 全为 0 的情况。我本地运行 x86_64 自测时正是这样成功匹配了 `NET_STATS_END`，但四个 byte counter 都是 0。建议让 `--test` 在 TCP/UDP packet 和 byte counter 没有按预期增长时返回错误，或至少让 QEMU success 条件匹配非零字节计数；否则这个测试不能防止 net_stats 的核心统计能力失效。


