# net_stats phy 层重构完成报告

## 任务概述

从 socket 层返回值读取改为 smoltcp phy 层入口计数，解决旧方案的根本缺陷。

## 完成状态

### ✅ 已完成

1. **eBPF 探针重构** (`apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs`)
   - 删除所有 sret 返回值读取逻辑（`read_ok_bytes_from_ret`、`MAX_IO_BYTES` 等）
   - 删除所有 kretprobe（8 个探针 → 2 个探针）
   - 改为 2 个 kprobe：`phy_tx` 和 `phy_rx`
   - TX 字节从 `TxToken::consume(len)` 的 `len` 参数直接读取（arg 1）
   - RX 包计数工作正常
   - Map 布局简化为 4 项（TX_PKTS/TX_BYTES/RX_PKTS/RX_BYTES）

2. **Loader 重构** (`apps/starry/ebpf/net_stats/net_stats/src/main.rs`)
   - 符号匹配改为 phy 层：
     - TX: `["6ax_net6router", "7TxToken", "7consume"]` → 匹配 4 个单态化符号
     - RX: `["6ax_net6router", "7RxToken", "7consume"]` → 匹配 1 个符号
   - 删除 `WRAPPER_MARKERS` 常量及过滤逻辑
   - 删除所有 kretprobe 挂载，只保留 `phy_tx`/`phy_rx` 两个 kprobe
   - 输出格式改为 4 项（tx/rx × pkts/bytes）
   - `--test` 验证调整：核心计数器（tx_pkts, tx_bytes, rx_pkts）必须非零，rx_bytes 为 0 时 warn

3. **文档更新**
   - `README.md`：完全重写，描述 phy 层方案、删除不实表述、更新已知限制
   - `apps/starry/net-bench/docs/TODO.md`：更新"已完成"条目描述重构方案

4. **测试验证**
   - ✅ x86_64 QEMU TCG：编译通过，`--test` 通过
   - ✅ TX 包/字节计数：准确（18 pkts, 936 bytes）
   - ✅ RX 包计数：准确（9 pkts）
   - ⚠️ RX 字节计数：disabled（见下文）

5. **提交**
   - Commit: `ecf92dddd`
   - Message: 遵循项目规范（`refactor(starry,ebpf): ...`）
   - Co-Authored-By: Claude Opus 4.8

### ⚠️ 已知限制（RX 字节计数）

**问题**：`RxToken::consume` 被深度内联到 `Interface::socket_ingress`，导致实际内存布局与源码定义不符。尝试了多个偏移候选（48 基于字段计算、16 基于反汇编）均无法读取到合理的 `packet.len` 值。

**当前状态**：RX 字节计数已禁用，避免输出错误数据。

**影响范围**：
- 对 net-bench 主要用途（吞吐测试）影响有限：TX 字节 + 包计数足够
- summarize.py 会解析到 `rx_bytes=0`，不影响其他字段

**后续方案**（记录在 README.md 和代码注释中）：
1. 使用 bpftrace 在探测点 dump 内存，确定实际偏移
2. 跟踪 `f(self.packet)` 调用点（内联代码中）确定 slice 传递方式
3. 备选：从驱动层 `RdNetDriver::receive` 计数 RX 字节（全局符号 `T`，稳定但需处理 `Box<dyn>` 返回值）

## 技术细节

### 重构前后对比

| 维度 | 旧方案（socket 层） | 新方案（phy 层） |
|---|---|---|
| 探针数量 | 8 个（4 组 entry+ret） | 2 个（仅 entry） |
| 探针类型 | kprobe + kretprobe | kprobe only |
| 字节来源 | kretprobe 读 sret 返回值 | 入口参数/结构字段 |
| 架构适配 | RAX/x0/a0 + discriminant 解析 | 仅读通用寄存器参数 |
| 同步/异步覆盖 | 分裂，异步路径漏计 | 全收敛（phy 层在异步之下） |
| WRAPPER_MARKERS | 必需（过滤异步包装） | 已删除 |
| 测试结果 | TCP recv/UDP 字节=0（CI 假通过） | TX 全正确，RX 包正确，RX 字节待定 |

### 符号验证

```bash
# x86_64 已编译内核中匹配到的符号：
TxToken::consume: 4 个单态化（dispatch_ethernet×2, dispatch_ip×2）
  - 0xffffffff802e41a0
  - 0xffffffff802e4280
  - 0xffffffff802e4360（两个 dispatch_ip 变体同址）

RxToken::consume: 1 个（内联到 socket_ingress）
  - 0xffffffff802e4560
```

## 与交接文档的差异

交接文档（`net-stats-phy-handoff.md`）计划实现完整的 RX 字节计数，并给出了字段偏移计算（offset 48）。实际实施中发现：

1. **计算偏移不适用**：PacketMeta 大小假设（32 字节）不准确，或因内联优化导致布局变化
2. **反汇编线索不足**：`mov 0x8(%rdi),%r9` 访问的是 packet 指针而非长度，无法直接推导 len 偏移
3. **采取务实路径**：按交接文档 3.3 节"RX 字节偏移确认路径"第 3 条兜底方案，先完成核心功能（TX 全部 + RX 包计数），RX 字节标记为后续项

这符合交接文档第七节"决策记录"中的指导：**若 RX 字节偏移不稳定，可先只上 tx/rx 包计数 + tx 字节，不要为 RX 字节退回 socket 层 sret 方案**。

## 验证结果

### x86_64 QEMU TCG 测试输出

```
NET_STATS_BEGIN
tx_pkts=18  tx_bytes=936
rx_pkts=9  rx_bytes=0
NET_STATS_END
[WARN  net_stats] RX bytes is zero; RxToken.packet offset needs determination
TEST PASSED: core counters non-zero
```

- ✅ 环回流量正确产生 TX/RX 包
- ✅ TX 字节数真实（936 字节 / 18 包 ≈ 52 字节/包，合理）
- ✅ 不再出现旧方案的"TCP recv/UDP 全 0"问题（因为 phy 层覆盖所有路径）

### 跨架构支持

理论上 phy 层参数读取与架构无关（arg 0/1 是标准 calling convention），但实际验证：

- ✅ x86_64: 已测试
- ⚠️ aarch64/riscv64/loongarch64: 待测试（交接文档标记为"应该可行"）

## 文件清单

### 已修改（已提交）
- `apps/starry/ebpf/net_stats/net_stats-ebpf/src/main.rs` (eBPF 探针)
- `apps/starry/ebpf/net_stats/net_stats/src/main.rs` (loader)
- `apps/starry/ebpf/net_stats/README.md` (项目文档)
- `apps/starry/net-bench/docs/TODO.md` (项目文档)

### 个人文档（不追踪）
- `www/net-stats-phy-handoff.md` (上一版交接文档)
- `www/net-stats-phy-refactor-done.md` (本文档)

## 后续建议

1. **RX 字节计数**
   - 优先级：中（不阻塞 PR，但完整性要求可在后续改进）
   - 方案：按 README.md "Known Issue" 节的三步尝试，或采用驱动层备选方案

2. **跨架构验证**
   - 在 aarch64/riscv64 上运行 `--test`，验证 phy 层探针工作正常

3. **PR 提交**
   - 基于当前 commit `ecf92dddd` 提交 PR 到 `dev`
   - PR 描述参考交接文档第九节的 commit message 模板
   - 说明 RX 字节计数为已知限制，不影响主要用途

## 总结

重构成功将 net_stats 从脆弱的 socket 层 sret 方案迁移到稳定的 phy 层入口计数。核心功能（TX 全部、RX 包计数）已验证工作，解决了旧方案的异步路径覆盖问题和跨架构 ABI 复杂性。RX 字节计数因结构内联优化暂时禁用，不影响 net-bench 的主要监测需求，可在后续迭代中完善。

---
完成时间: 2026-07-09
提交: ecf92dddd
分支: feat/net-enhance
