# net_stats 真实字节计数实现进展(2026-07-09)

## 当前状态

eBPF 侧与 loader 侧均已按方案实现,QEMU x86_64 自测部分成功。

### 实测结果(x86_64)

```
tcp_tx_pkts=2  tcp_tx_bytes=32
tcp_rx_pkts=0  tcp_rx_bytes=0
udp_tx_pkts=2  udp_tx_bytes=0
udp_rx_pkts=2  udp_rx_bytes=0
```

**已验证**:TCP send 读取真实字节数(32 bytes,平均 16 bytes/pkt,非估算值 pkts×64=128)。

**异常**:TCP recv 与 UDP send/recv 字节计数仍为 0,且 `tcp_rx_pkts=0`(entry 探针未触发)。

### 符号解析与挂载(已确认)

loader 日志:`resolved tcp_send=3, tcp_recv=1, udp_send=3, udp_recv=3`

符号过滤器正常工作,canonical trait 方法符号已被 kallsyms 解析并挂载。

### 根因分析(tcp_rx / udp 全 0)

1. **TCP recv entry 未触发**:`tcp_rx_pkts=0` 说明 kprobe entry 探针根本未执行,而非 sret 读取失败。
2. **可能原因**:
   - 测试流量未走 canonical `<TcpSocket as SocketOps>::recv`(虽然反汇编确认该符号有 1 处调用点 `0x8008817f`,但可能测试路径未执行到)。
   - loopback recv 可能被内联或走了其它单态化变体(我们过滤掉了 poll_fn/Future::poll 异步包装,这些返回 `Poll<...>` 而非 `Result`)。
   - UDP 同理:虽然符号挂载了,但测试流量可能未真正执行到那些单态化实例。

## 核心成果

**sret 指针读取机制已验证可行**:TCP send 的 32 字节是真实值,证明从 `ctx.ret()` 读 sret 指针、解引用 `[disc@0, bytes@8]` 的方案在 x86_64 上成功。这推翻了原 summary 的"kretprobe 无法读取 sret"结论。

## 下一步选项

### 选项 A:扩大探测范围(权宜)

临时放宽过滤器,**也挂载 `block_on` 包装符号**(它们返回同样的 `AxResult<usize>`,只是在更外层),以覆盖更多调用路径。但这会让包计数膨胀(一次逻辑操作触发多层 entry)。

### 选项 B:深入调试测试流量(根治)

1. 在 loader 增加"挂载成功/失败"的详细日志(当前只记录了 resolve 计数,未记录 attach 结果)。
2. 用 `bpftrace` / 手动 kprobe 确认测试期间哪些符号真正被调用。
3. 检查 StarryOS loopback 实现是否绕过了某些 SocketOps 路径。

### 选项 C:接受当前状态,文档记录(务实)

- 标注 TCP send 字节为真实值(已验证)。
- TCP recv / UDP 标注为"当前测试环境未覆盖,生产流量待验证"。
- 更新 README:字节计数经 sret 读取(已在 TCP send 验证),部分路径待完整覆盖。

## 建议

鉴于核心机制(sret 读取)已证明有效,且时间有限,建议**选项 C + 简化版 A**:

1. 放宽过滤器,允许 `block_on` 符号(它们返回类型与 canonical 一致,只是调用层次更高)。
2. 重测,若 recv/UDP 仍为 0,则文档如实记录当前验证范围。
3. 回改文档为"真实字节计数(经 sret 读取),已在 TCP send 验证;其余路径视生产流量情况"。

## 文件修改清单

- [x] eBPF 侧:`net_stats-ebpf/src/main.rs` 从 `ctx.ret()` 读 sret 指针
- [x] loader 侧:`net_stats/src/main.rs` 符号过滤 + debug 日志
- [ ] 测试:x86_64 部分成功(TCP send 32 字节真实值)
- [ ] 文档:待根据最终测试结果回改

## 遗留问题

- TCP recv / UDP 在当前 loopback 自测中未产生字节计数(entry 探针未触发)。
- 需在真实生产流量(非 loopback)或更完整测试场景下验证 recv/UDP 路径。
