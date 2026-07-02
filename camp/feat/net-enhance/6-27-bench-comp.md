
## Starry 网络性能测试与文档计划的差距分析

基于 `www/` 规划文档（methodology + qemu-benchmark-plan）与 `apps/starry/net-bench` 当前实现的对比：

---

## 一、测试拓扑差距

### 1.1 规划要求（methodology §0, qemu-plan §0-§2）

| 优先级 | 拓扑 | 状态 | 用途 |
|--------|------|------|------|
| **主力** | QEMU+TAP+**vhost-net** | ❌ **缺失** | 全套性能指标、优化对比 |
| 降级 | QEMU+TAP（无vhost） | ✅ **已实现** | 功能/趋势兜底 |
| 禁用 | QEMU+SLIRP | ⚠️ **当前默认** | 仅冒烟，禁用于压测 |
| 未来 | 物理机+真实网卡 | 不承担 | 终极校准（范围外） |

### 1.2 当前实现状态

**`apps/starry/net-bench` 支持的拓扑**：
```bash
# 已实现
├── SLIRP (qemu-aarch64.toml)           当前默认，smp=1
├── SLIRP (qemu-aarch64-smp4.toml)      smp=4
└── TAP (qemu-aarch64-tap.toml)         无vhost，需手动配置tap0
```

**关键配置对比**：

| 特性 | 规划要求（qemu-plan §2.3） | 当前实现 | 差距 |
|------|---------------------------|----------|------|
| vhost-net | `-netdev tap,vhost=on` | ❌ 未配置 | **主力拓扑缺失** |
| 多队列 | `mq=on,queues=4,vectors=10` | ❌ 未配置 | 无法验证多队列改造 |
| offload开关 | `csum/gso/tso` 显式控制 | ❌ 未配置 | 无法验证offload打通 |
| 绑核 | `taskset -c` + 固定核 | ❌ 未实现 | 噪声未压制 |
| WSL2降噪 | `.wslconfig` 固定资源 | ❌ 未集成 | 方差控制缺失 |

**影响**：
- ❌ 当前 TAP 配置**不符合主力测试拓扑要求**（无vhost吞吐被限制）
- ❌ SLIRP 默认场景**违反规划纪律**（"禁用于压测"但当前是默认）
- ⚠️ 无法验证 `performance-analysis.md §2.4` 的校验和offload打通效果
- ⚠️ 无法验证 `performance-analysis.md §3.1` 的多队列/RSS改造

---

## 二、测试工具覆盖差距

### 2.1 规划六维指标（methodology §1）

| 维度 | 规划工具 | 当前实现 | 覆盖度 |
|------|----------|----------|--------|
| **吞吐** | iperf3 TCP `-P`/`-R`/`--bidir` | ✅ **已实现** tcp1/tcp4/tcp1r | 60%（缺bidir） |
| **PPS/小包** | netperf UDP_RR、iperf3 `-u -l 64` | ✅ **已实现** udp64 | 50%（仅iperf3） |
| **延迟** | netperf TCP_RR/UDP_RR、sockperf | ❌ **缺失** | 0% |
| **连接速率** | netperf TCP_CRR、nginx+wrk | ❌ **缺失** | 0% |
| **CPU效率** | perf stat、周期计数器埋点 | ⚠️ **部分**（net_stats eBPF） | 20% |
| **多核扩展** | 全套 × smp1/smp4 × taskset | ⚠️ **框架已有**（未绑核） | 40% |

### 2.2 当前 `net-bench-common.sh` 覆盖

```bash
run_test tcp1   -P 1           # TCP 单流上行 ✅
run_test tcp4   -P 4           # TCP 4流上行 ✅
run_test tcp1r  -P 1 -R        # TCP 单流下行 ✅
run_test udp1g  -u -b 1G       # UDP 大包吞吐 ✅
run_test udp64  -u -b 0 -l 64  # UDP 64B小包 PPS ✅
```

### 2.3 缺失的关键测试

| 缺失项 | 工具 | 为何重要 | 对应文档 |
|--------|------|----------|----------|
| TCP/UDP RR延迟 | netperf | 暴露poll/yield/调度尾延迟 | methodology §1 |
| TCP_CRR 短连接 | netperf | 压listen_table/端口分配锁 | analysis §3.2 |
| nginx+wrk HTTP | wrk | 真实应用场景说服力 | methodology §2.1 |
| 双向流量 | iperf3 `--bidir` | 收发路径同时压测 | methodology §1 |
| 裸帧吞吐 | bwbench_client | 隔离驱动层vs协议栈层 | methodology §2.3 |
| 火焰图 | qperf | 热点可视化（慢在哪） | methodology §3.2 |
| Linux基线对比 | 同工具同拓扑 | 量化与Linux差距 | methodology §4.1 |

**影响**：
- ❌ 无法验证 `analysis §1.3` poll模型改造对延迟的影响
- ❌ 无法验证 `analysis §3.2` listen_table HashMap优化的连接速率提升
- ❌ 无法量化 `analysis §2.1/2.2` 多拷贝与加锁的CPU代价（cycles/packet）

---

## 三、观测能力差距

### 3.1 规划要求（methodology §3, qemu-plan §5）

#### Linux侧观测（已部分实现）
| 工具 | 规划用途 | 当前实现 | 状态 |
|------|----------|----------|------|
| perf stat | cycles/byte、IPC、cache-miss | ❌ 未集成 | 缺失 |
| bpftrace/bcc | 协议栈tracepoint、丢包归因 | ❌ 未集成 | 缺失 |
| 火焰图 | perf record热点可视化 | ❌ 未集成 | 缺失 |

#### Starry侧观测（部分实现）
| 需求 | 规划实现 | 当前实现 | 完成度 |
|------|----------|----------|--------|
| socket层计数 | per-socket rx/tx/drop | ✅ **net_stats eBPF** | 70% |
| payload字节数 | send/recv返回值 | ✅ **net_stats eBPF** | 70% |
| 拷贝次数计数 | to_vec/copy_from_slice埋点 | ❌ 未实现 | 0% |
| 阶段cycles采样 | poll_interfaces前后取差 | ❌ 未实现 | 0% |
| 锁竞争观测 | lockdep SERVICE/SOCKET_SET | ❌ 未集成 | 0% |
| 丢包归因 | 按原因分类计数 | ❌ 未实现 | 0% |
| 火焰图 | qperf + harness_kit | ❌ 未集成 | 0% |

### 3.2 `net_stats` eBPF 当前定位

**实现范围**（EBPF_NET_STATS.md §0）：
- ✅ TCP/UDP send/recv 调用计数
- ✅ payload字节数累计
- ✅ 输出结构化标记（NET_STATS_BEGIN/END）
- ⚠️ 仅x86_64 ABI验证（aarch64/riscv64待适配）

**明确限制**：
- ❌ 不是完整性能benchmark
- ❌ 不是网卡层/IP层统计
- ⚠️ `*_pkts` 实际是"调用次数"而非真实包数
- ⚠️ 定位：**net-bench的诊断辅助信号**

**影响**：
- ✅ 可辅助判断"内核路径是否命中"（符合设计目标）
- ❌ 无法提供 `methodology §1` 要求的核心KPI：**cycles/packet、cycles/byte**
- ❌ 无法归因"慢在哪"（缺火焰图集成）

---

## 四、统计严谨性差距

### 4.1 规划纪律（methodology §4.2, qemu-plan §3.4）

| 纪律 | 规划要求 | 当前实现 | 状态 |
|------|----------|----------|------|
| 迭代次数 | ≥5次，mean±stddev | ✅ **已实现**（1 warmup+5测量） | 100% |
| 噪声识别 | 相对标准差>10%标注[NOISY] | ✅ **summarize.py** | 100% |
| warmup过滤 | 自动丢弃warmup迭代 | ✅ **标记+解析** | 100% |
| 跨boot重复 | `--repeat N` 累积方差 | ✅ **run.sh --repeat** | 100% |
| 环境指纹 | uname/QEMU/commit自动记录 | ✅ **fingerprint-*.txt** | 100% |
| 固定测试时长 | 如 `-t 30` | ⚠️ 当前 `-t 10` 偏短 | 60% |
| 绑核固定 | taskset + guest内绑核 | ❌ 未实现 | 0% |
| WSL2降噪配置 | .wslconfig文档化 | ⚠️ 规划文档提及但未集成 | 30% |

### 4.2 已实现的优势

**`summarize.py` 统计引擎**（340行）：
- ✅ 自动解析 iperf3 JSON
- ✅ 过滤 warmup=1 迭代
- ✅ 计算 mean±stddev
- ✅ 相对标准差>10%自动标注
- ✅ 支持跨多次重启合并
- ✅ JSON/text双输出格式
- ✅ 无外部依赖（仅标准库）

**差距**：
- ❌ 仅支持 iperf3 JSON，不支持 netperf/wrk 输出
- ❌ 未集成 Linux基线对比（三方柱状图）
- ❌ 未计算 cycles/packet 等派生指标

---

## 五、报告结构差距

### 5.1 规划要求（methodology §4）

**实验矩阵每格记录**：
```
吞吐 Gbps（mean±stddev）
PPS、延迟 P50/P99/P999
CPU: cycles/byte、cycles/packet、整体CPU%
丢包/重传（按原因）
多核扩展比
```

**呈现方式**：
- 三条对比线：Linux基线 / Starry优化前 / Starry优化后
- 多核扩展曲线（吞吐 vs 核数）
- 火焰图并排（Linux perf vs Starry qperf）
- 归因表（指标变化 → 改动映射）

### 5.2 当前实现

**`summarize.py` 输出格式**：
```
TCP 1-stream (uplink):    93.1 ± 2.4 Mbit/s
TCP 4-stream (uplink):    93.9 ± 1.8 Mbit/s
UDP 64B (PPS):           12345 ± 678 pkt/s
```

**差距**：
- ❌ 无 Linux基线对比列
- ❌ 无 cycles/packet、CPU效率指标
- ❌ 无延迟分布（P50/P99/P999）
- ❌ 无丢包/重传归因
- ❌ 无多核扩展比计算
- ❌ 无火焰图集成
- ❌ 无可视化图表生成

---

## 六、差距总结与优先级

### 6.1 按影响分级

#### 🔴 **阻塞性差距**（无法开展主力测试）
1. **QEMU+TAP+vhost 拓扑缺失**
   - 当前TAP无vhost，SLIRP为默认
   - 影响：所有吞吐数据不符合主力测试要求
   - 对应：qemu-plan §2.3

2. **Linux基线对比缺失**
   - 无法量化"Starry达到Linux的百分比"
   - 影响：无法证明优化有效性
   - 对应：methodology §4.1

3. **cycles/packet 观测缺失**
   - 核心KPI无法测量
   - 影响：无法验证"多拷贝+软校验和+每包加锁"的代价
   - 对应：methodology §1、analysis §2.1

#### 🟠 **关键差距**（限制测试完整性）
4. **延迟测试缺失**（netperf RR、sockperf）
   - 无法验证poll模型改造效果
   - 对应：analysis §1.3

5. **连接速率测试缺失**（TCP_CRR、nginx+wrk）
   - 无法验证listen_table优化
   - 对应：analysis §3.2

6. **火焰图未集成**（qperf + perf）
   - 无法归因"慢在哪"
   - 对应：methodology §3.2

7. **绑核与降噪未实施**
   - 方差控制不足
   - 对应：qemu-plan §3.2-3.3

#### 🟡 **增强差距**（提升可信度）
8. **offload开关控制缺失**
   - 无法验证校验和offload打通
   - 对应：analysis §2.4

9. **多队列配置缺失**
   - 无法验证多队列/RSS改造
   - 对应：analysis §3.1

10. **Starry侧细粒度观测不足**
    - 拷贝计数、阶段cycles、锁竞争、丢包归因
    - 对应：methodology §3.2

### 6.2 完成度评估

| 维度 | 完成度 | 核心缺失 |
|------|--------|----------|
| **测试拓扑** | 30% | vhost-net、降噪纪律 |
| **工具覆盖** | 40% | 延迟/连接速率/CPU效率 |
| **观测能力** | 25% | cycles采样、火焰图、锁竞争 |
| **统计严谨性** | 70% | 绑核、Linux基线对比 |
| **报告结构** | 20% | 三方对比、派生指标、可视化 |
| **整体** | **35%** | - |

---

## 七、对齐路线图

### 短期（1-2周）- 解除阻塞
1. 实现 QEMU+TAP+vhost 配置（qemu-plan §2.3）
2. 添加 Linux基线测试脚本（同拓扑、同二进制）
3. 集成 perf stat 采集 cycles/instructions

### 中期（2-4周）- 补齐关键维度
4. 添加 netperf TCP_RR/UDP_RR 延迟测试
5. 添加 netperf TCP_CRR 短连接测试
6. 集成 qperf 火焰图生成
7. 实现绑核与WSL2降噪配置

### 长期（1-2月）- 完整体系
8. 添加 nginx+wrk HTTP测试
9. Starry侧细粒度计数（拷贝/cycles/锁/丢包）
10. 三方对比报告生成器（含可视化）
11. offload/多队列配置支持

---

## 八、当前实现的优势

尽管有差距，`apps/starry/net-bench` 已建立的基础仍很扎实：

✅ **结构化测量框架**：BEGIN/END标记、warmup过滤、多迭代统计  
✅ **噪声识别纪律**：相对标准差>10%自动标注  
✅ **环境指纹记录**：满足可复现性要求  
✅ **跨启动重复**：`--repeat N` 累积方差  
✅ **eBPF辅助观测**：net_stats提供内核侧信号  
✅ **文档完整**：README 114行、EBPF_NET_STATS 349行  
✅ **多场景支持**：SLIRP/TAP、smp1/smp4  
✅ **自动化闭环**：run.sh → xtask → summarize.py  

**核心价值**：提供了从"功能冒烟"到"性能基线"的**80%基础设施**，剩余20%是关键性能指标的补齐。
