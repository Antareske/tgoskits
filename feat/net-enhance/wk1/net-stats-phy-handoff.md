# net_stats 重构工作交接文档:从 socket 层返回值读取改为 phy 层入口计数

## 一、原始需求与目标

核心目的:**为 starry 网络性能测试(net-bench)提供 guest 侧的网络监测 eBPF**,产出可解析的 tx/rx 包数与字节数,供 summarize.py 汇总、与 iperf3 交叉验证。

net_stats 在 net-bench 中的定位是**辅助观测手段**(主测量为 iperf3),见 `apps/starry/net-bench/docs/TODO.md`。

## 二、为什么要重构(背景与决策依据)

### 2.1 旧方案(当前工作树中未提交的 sret 方案)的困境

先前一版工作把字节计数实现为:在 `<TcpSocket/UdpSocket as SocketOps>::send/recv` 的 **kretprobe** 处读返回值 `AxResult<usize>` 的 sret 指针并解引用。经查证:

- **sret 机制本身可行**:反汇编 x86_64 内核确认 TCP send / UDP send 尾声均为 `mov %rbp,%rax; ...; ret`,返回瞬间 sret 指针在 RAX,`ctx.ret()` 能读到。TCP send 实测得到真实 32 字节。旧代码里"kretprobe 读不到 sret"的注释是误诊。
- **但该方案有根本缺陷**:
  1. 需按架构适配返回寄存器(RAX/x0/a0)、判 `AxResult` discriminant、防陈旧指针,glue 复杂且随编译器/starry 升级易腐烂。
  2. socket 层有**同步 canonical 方法**与**异步包装层**(`block_on`/`poll_fn`/`Future::poll`)两条路径。异步包装层返回 `Poll<AxResult<usize>>`,布局不同,必须用 `WRAPPER_MARKERS` 过滤掉。
  3. 过滤后只挂 canonical 方法,而 **loopback 测试里 TCP recv、UDP 走的正是被过滤掉的异步路径** → `tcp_rx_pkts=0`、UDP 字节=0 → `--test` 失败(要求 4 项字节全非零)。

### 2.2 已提交版本(HEAD)的真实情况

HEAD(commit 15b2a0a71 及其前序)用 `packets × 64` 估算字节,并挂载**全部 ~19 个匹配符号**(含异步包装层)。CI 能过,是因为包装层在 recv/UDP 路径会触发 → 产生非零(假)字节,但代价是**包计数被夸大**(一次逻辑操作多层 entry)。即 HEAD 也不正确,只是"看起来通过"。

### 2.3 关键结论:探点选错了层

问题根源是**在 socket 层探测**——那里既要读返回值(ABI 麻烦),又有同步/异步分裂(覆盖不全)。

调研 starry 网络栈(smoltcp 0.13.1)后确认:

- smoltcp socket 层**无累计计数器**,只有 `send_queue()`/`recv_queue()`(瞬时队列深度)、容量、状态,拿不到累计收发字节;TCP 序列号未 `pub`。
- 无可直接复用的开源 eBPF(bcc/bpftrace/Cilium 等均依赖 Linux 稳定内核接口 + 完整 BTF/helper/verifier,而 starry 是自研受限 eBPF 运行时 + Rust v0 mangled 符号,不适用)。
- **真正的全流量收敛点在 smoltcp phy 层**:`net/ax-net/src/router.rs` 中 `impl smoltcp::phy::Device for Router` 的 `TxToken::consume` / `RxToken::consume`。所有 IP 帧(无论应用层走同步还是异步)必经此处。

## 三、重构方案(phy 层入口计数)

### 3.1 方案要点

把探点从 socket 层(读返回值)下移到 phy 层(读入口参数):

| 维度 | 旧(socket 层) | 新(phy 层) |
|---|---|---|
| 探针 | 8 个(4 组 entry+ret) | 2 个 kprobe(仅 entry) |
| 字节来源 | kretprobe 返回值 sret 指针 | **入口参数/结构字段** |
| 架构适配 | RAX/x0/a0 各不同 + discriminant | 仅读通用寄存器参数 |
| 同步/异步 | 分裂,异步漏计 | phy 层在异步机制之下,全收敛 |
| WRAPPER_MARKERS | 必需 | **可删除** |
| kretprobe | 需要 | **全部删除** |
| 语义 | 应用层 payload | 链路帧字节(含 IP/TCP 头),更贴合吞吐测试 |

**唯一代价**:phy 层看到的是 IP 帧,默认分不清 TCP/UDP。net-bench 关注 tx/rx 方向 + 总字节 + pps,足够;若需分协议,可在 eBPF 内读 IP 头 version/protocol 字节(可选增强)。

### 3.2 已核实的关键事实(基于 x86_64 已编译内核 target/x86_64-unknown-linux-musl/release/starryos 反汇编)

**TX 探点** `TxToken::consume`:
- 签名 `fn consume<R,F>(self, len: usize, f: F) -> R`,`self` 是 `TxToken(&mut RouterPacketBuffer)`(单指针)。
- x86_64 下 `self`=rdi(arg0),**`len`=rsi(arg1)**,eBPF 用 `ctx.arg(1)` 直接取字节数,无需返回值。
- 符号片段 `6ax_net6router` + `7TxToken` + `7consume` 精确命中 **4 个单态化**(dispatch_ethernet×2、dispatch_ip×2),已验证无误伤;同一 kprobe 挂到 4 个符号,入口都读 rsi。
  - 4 个符号地址:0x802e41a0 / 0x802e4280 / 0x802e4360(两个 dispatch_ip 变体同址)。

**RX 探点** `RxToken::consume`:
- 签名 `fn consume<R,F>(self, f: F) -> R`,**无 len 参数**;字节数需从 `self` 结构读。
- 结构定义(`net/ax-net/src/router.rs:1059`):
  ```rust
  pub struct RxToken<'a> {
      interface_id: InterfaceId,   // 偏移待定
      packet_meta: PacketMeta,     // 偏移待定
      packet: &'a [u8],            // 胖指针 (ptr, len)
  }
  ```
- 字节数 = `packet` 的 slice len。**⚠ 该字段在结构内的确切偏移需运行时/反汇编确认**:反汇编显示此 consume 被内联进 `Interface::socket_ingress`,`self` 布局不能直接照搬。入口反汇编首指令为 `mov 0x8(%rdi),%r9`(self+8 是指针),但不能据此断定 len 偏移。
- 符号片段 `6ax_net6router` + `7RxToken` + `7consume` 命中 **1 个符号**(0x802e4560)。

### 3.3 RX 字节偏移的确认路径(实施时必做)

按优先级:
1. **反汇编定位**:对 RxToken::consume(0x802e4560)详细反汇编,或对未内联的独立 `RxToken::consume` 实例分析 `self` 布局,找到 `packet.len` 相对 self 的偏移(InterfaceId + PacketMeta 大小之后 + 8)。
2. **源码推算 + 验证**:查 `InterfaceId`、`PacketMeta`(smoltcp::phy::PacketMeta)的大小与对齐,算出 `packet` 字段偏移,再用运行时数值验证。
3. **兜底**:若 RxToken len 偏移不稳定,RX 字节改从别处取——如 `snoop_tcp_packet(buf: &[u8], ...)`(0x… 入口 rdi=ptr, rsi=len,仅 TCP)或驱动层 `RdNetDriver::receive`(全局导出符号 `T`,但返回 `Box<dyn NetRxBuffer>`,又回到读返回值,不优先)。RX 包计数无需字节即可先行。

### 3.4 备选探点(已评估,非首选)

- **驱动层** `RdNetDriver::transmit`/`receive`(`net/ax-net/src/device/driver.rs`):符号是**全局导出 `T`**,最稳定,各 1 个无单态化爆炸。但 transmit 的 buf 长度要走 vtable(`call *0x28(%rdx)`)读、receive 返回 `Box<dyn>` 又是返回值问题。作为 phy 层不可行时的备选。

## 四、实施计划(具体改动清单)

### 4.1 eBPF 侧 `apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs`

1. 删除 `read_ok_bytes_from_ret`、`MAX_IO_BYTES`、`bpf_probe_read_kernel` 相关 sret 逻辑与长注释。
2. map 索引简化(二选一):
   - 方案甲(推荐):4 项 `TX_PKTS/TX_BYTES/RX_PKTS/RX_BYTES`,`MAP_SIZE=4`。
   - 方案乙:保留 8 项、UDP 项留空,减少 loader/summarize 改动。建议先甲,同步改 loader 与 summarize.py。
3. 探针改为 2 个 kprobe(删除全部 kretprobe):
   ```rust
   #[kprobe]
   pub fn phy_tx(ctx: ProbeContext) -> u32 {
       // TxToken::consume(self=arg0, len=arg1)
       let len: usize = ctx.arg(1).unwrap_or(0);
       add_to(TX_PKTS, 1);
       add_to(TX_BYTES, len as u64);
       0
   }
   #[kprobe]
   pub fn phy_rx(ctx: ProbeContext) -> u32 {
       // RxToken::consume(self=arg0); packet slice len 在 self 固定偏移(3.3 确认)
       add_to(RX_PKTS, 1);
       // let len = 读 self 偏移处 slice len; add_to(RX_BYTES, len);
       0
   }
   ```
   注:`ctx.arg(1)` 的返回类型需按 aya API 处理(通常 `Option<usize>`)。

### 4.2 loader 侧 `apps/starry/ebpf/net_stats/net_stats/src/main.rs`

1. `resolve_symbols` 匹配片段改为:
   - TX:`["6ax_net6router", "7TxToken", "7consume"]`
   - RX:`["6ax_net6router", "7RxToken", "7consume"]`
2. **删除 `WRAPPER_MARKERS` 常量及过滤逻辑**(phy 层无异步包装层问题)。
3. `attach_all!` 保留(TX 挂 4 个单态化符号,RX 挂 1 个);**删除所有 kretprobe/ret 程序挂载**,只留 `phy_tx`/`phy_rx` 两个 kprobe。
4. 常量与 `print_stats`、`--test` 校验按 4 项(tx/rx pkts/bytes)重排。
5. `--test` 校验:环回流量必过 phy 层,4 项(至少 tx/rx pkts + tx bytes)应非零;RX bytes 视 3.3 结果决定是否纳入硬校验(未确认前可先 warn)。

### 4.3 输出解析 `apps/starry/net-bench/core/summarize.py`

- 若采方案甲(4 项),同步调整 NET_STATS 块解析字段(原 tcp/udp × tx/rx × pkts/bytes 8 字段 → tx/rx × pkts/bytes 4 字段)。核对 `print_stats` 输出格式与 summarize.py 正则一致。

### 4.4 文档(项目文档,须与实现对齐)

- `apps/starry/ebpf/net_stats/README.md`:
  - Features / 字段说明改为 phy 层 tx/rx 语义(链路帧字节,不分协议;或注明分协议为可选)。
  - "Packet vs Byte Counters" / "Known Limitations":重写为描述 phy 层入口计数机制。
  - **修正现有不实表述**:当前 README 声称 aarch64/riscv64 "✅ Fully tested and working" 属不实(仅 x86_64 跑过);"Testing" 节仍称字节为 "estimated values" 已过时。重构后按真实验证范围重写。
- `apps/starry/net-bench/docs/TODO.md`:更新"已完成"中 net_stats 字节计数条目为 phy 层方案;检查"中优先级/eBPF net_stats 集成修正"是否受影响。

## 五、验证方式

- 环境:本机 QEMU 为 **TCG(无 /dev/kvm,无 ostool)**,完整重编+启动较慢但可行。
- 命令:`cargo xtask starry app qemu --test-case ebpf/net_stats --arch x86_64`
- 预期:phy 层收敛,loopback 下 tx/rx pkts、tx bytes 均非零(不再出现 recv/UDP 为 0)。
- 跨架构:aarch64/riscv64 同理(phy 层参数读取与架构无关),按需 `--arch aarch64` 复验;loongarch64 有 QEMU virtio 已知问题(与本改动无关)。
- RX bytes:按 3.3 确认偏移后单独验证数值正确性。

## 六、当前状态(交接时点)

- 工作树:`apps/starry/ebpf/net_stats/{README.md, net_stats-ebpf/src/main.rs, net_stats/src/main.rs}`、`apps/starry/net-bench/docs/TODO.md` 有**未提交改动**,内容是 **2.1 的旧 sret 方案**(CI 会失败,含不实文档)。
- **重构尚未开始编码**。接手者应:先 `git stash` 或直接在此基础上按第四节改写(旧 sret 方案整体废弃,不必保留)。
- HEAD 的估算方案在 git 历史中(commit 15b2a0a71 等 5 个 commit,均在 dev 之后本分支分叉提交)。

## 七、决策记录(供接手者判断是否推进)

- 已与需求方确认:**同意放弃 socket 层返回值读取,改 phy 层入口计数**。理由见第二节:更简单、更稳、CI 可真实通过、语义更贴合吞吐测试,且消除会腐烂的 ABI glue。
- 若接手后发现 phy 层 RX 字节偏移不稳定,可先只上 tx/rx 包计数 + tx 字节,RX 字节标为后续项;不要为 RX 字节退回 socket 层 sret 方案。

## 八、相关文件清单

### 待改(项目文件)
- `apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs`(eBPF 探针)
- `apps/starry/ebpf/net_stats/net_stats/src/main.rs`(loader)
- `apps/starry/net-bench/core/summarize.py`(若改 map 布局)
- `apps/starry/ebpf/net_stats/README.md`(项目文档)
- `apps/starry/net-bench/docs/TODO.md`(项目文档)

### 只读参考(理解探点)
- `net/ax-net/src/router.rs`(phy Device/TxToken/RxToken 实现,行 1006-1116)
- `net/ax-net/src/device/driver.rs`(驱动层备选探点,行 120-300)

### 个人文档(项目不追踪)
- `www/net-stats-handoff.md`(上一版 sret 方案交接,已被本方案取代)
- 本文件 `www/net-stats-phy-handoff.md`

### 内存文档
- `/home/ubuntu/.claude/projects/-workspace-tgoskits/memory/net-stats-sret-abi.md`(sret 根因调查,仍有效作背景,但探点结论已被 phy 层方案取代)

## 九、提交建议(重构完成后)

```
refactor(starry,ebpf): count net_stats at smoltcp phy layer instead of socket returns

Move net_stats probes from <Socket as SocketOps>::send/recv (which required
reading the AxResult<usize> sret pointer at kretprobe and split across
sync/async paths) down to the smoltcp phy layer TxToken/RxToken::consume in
ax_net::router, where every IP frame converges.

TX byte length is the `len` scalar argument at entry; RX length is read from
the RxToken packet slice. This removes all per-arch return-register handling,
the WRAPPER_MARKERS async-wrapper filtering, and every kretprobe, and fixes the
zero TCP-recv/UDP byte counts caused by async paths bypassing the canonical
socket methods.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```

注意提交时避开 AGENTS.md/CLAUDE.md/.claude/.ocr 等与分支工作无关文件。

交接完毕。
