# Starry 网络增强文档更新说明

本次更新基于 dev 分支最新进度（commit 6a857920），反映已完成的架构重构和待推进的性能优化工作。

## 主要变更

### 1. 已完成的关键重构（标注为 ✅）

#### 1.1 poll 模型重构（#1340, #1278）
- ✅ 引入专属 net-poll worker，数据面与 syscall 解耦
- ✅ socket 方法通过 `request_poll()` + waker 异步推进协议栈
- ✅ `DEFERRED_POLL_WAKES` 机制避免在持锁时触发 waker
- ✅ IRQ-safe 延迟通知支持

**影响**：消除了"同步内联 poll"导致的全局串行化问题，为后续性能优化打好基础。

#### 1.2 listen_table 优化（#1340）
- ✅ 从 65536 个数组槽位改为 HashMap 索引
- ✅ 端口查找不再线性扫描

**影响**：降低端口分配与查重开销，改善 accept 路径性能。

#### 1.3 并发与锁语义文档（#1340）
- ✅ 新增 878 行锁顺序、竞态窗口、deferred poll waker 机制说明
- ✅ 位置：`docs/docs/architecture/net/locks.md`

**影响**：为后续多核优化和锁拆分工作提供明确的设计文档。

#### 1.4 多接口支持（#1244）
- ✅ per-interface 路由表、DNS
- ✅ 孤儿 TCP 回收、环回快速路径
- ✅ SO_BINDTODEVICE、dual-net 测试用例

**影响**：完善网络功能完整性，支撑复杂网络拓扑场景。

#### 1.5 virtio-net 队列安全修复（#1392）
- ✅ 修复多核竞态，保证 virtio 队列访问序列化

**影响**：提升多核环境下的稳定性。

### 2. 当前完成度评估

| 阶段 | 完成度 | 已完成项 | 待推进项 |
|------|--------|---------|---------|
| 阶段一：稳定基线 | **40%** | poll 模型重构、listen_table 优化 | 批处理、per-socket 计数、panic 清理、缓冲扩容 |
| 阶段二：单核效率 | **10%** | poll 解耦为后续优化打好基础 | 去拷贝、buffer 池、offload、窗口调整 |
| 阶段三：多核扩展 | **20%** | poll 线程解耦、listen_table HashMap | SOCKET_SET 锁拆分、多队列/RSS |

**综合完成度：约 30%**

### 3. 文档更新内容

#### 3.1 starry-net-enhancement-overview.md
- 补充"已完成的关键重构"章节
- 更新"瓶颈画像"，标注已改善和待推进项
- 更新"优化路线"表格，标注当前完成度
- 新增"后续推进计划"章节

#### 3.2 starry-net-performance-analysis.md
- 更新架构图，反映 net-poll worker 和 deferred waker
- 标注已完成项（✅ 已完成）和待推进项
- 更新 §2.3 poll 模型章节，说明重构成果
- 更新 §3.1 多核扩展瓶颈，区分已改善和待解决
- 更新 §3.3 waker 机制，标注已优化
- 更新 §6 工作路线，按三阶段标注完成度

#### 3.3 starry-net-benchmark-methodology.md
- 更新文档头部，标注基于 2024-06 dev 分支
- 补充当前可观测性基础设施状态
- 标注 lockdep 已就绪，per-socket 计数器待实现

#### 3.4 starry-net-qemu-benchmark-plan.md
- 更新文档头部，反映 poll 模型重构后的测试要求
- 更新 §5.2 Starry 侧观测点，标注当前状态和待实现项

### 4. 核心待办事项（优先级排序）

#### 短期（1-2 周）
1. 清理数据面 panic 路径（`router.rs:224,245`、`service.rs:451` 等）
2. 实现 RX 批处理（`RX_PREFETCH_TARGET` 改为 32/64）
3. 添加 per-socket/device 计数器（rx/tx/drop/retrans）

#### 中期（2-4 周）
4. 去拷贝优化（driver buffer 所有权传递，去掉 `to_vec()`）
5. virtio TX 去 staging 拷贝
6. 打通校验和 offload（virtio-net `VIRTIO_NET_F_CSUM/GSO`）

#### 长期（1-2 月）
7. 拆分 `SOCKET_SET` 全局锁为 per-socket 或分片锁
8. 驱动多队列 + RSS
9. 搭建 QEMU+vhost 测试环境并建立 Linux 基线

### 5. 关键代码位置参考

| 组件 | 位置 | 说明 |
|------|------|------|
| 全局锁定义 | `net/ax-net/src/lib.rs:116-130` | SERVICE、SOCKET_SET、LISTEN_TABLE、poll worker |
| listen_table 实现 | `net/ax-net/src/listen_table.rs` | HashMap 索引、backlog 管理 |
| poll worker 逻辑 | `net/ax-net/src/service.rs` | net-poll worker 入口、deferred waker |
| 并发文档 | `docs/docs/architecture/net/locks.md` | 878 行锁顺序与竞态说明 |
| RX 批处理常量 | `net/ax-net/src/device/driver.rs` | RX_PREFETCH_TARGET = 1 |
| 拷贝热点 | `net/ax-net/src/device/driver.rs`<br>`drivers/net/rd-net/src/lib.rs` | to_vec()、copy_from_slice |

### 6. 参考 PR

- #1340: poll 模型重构、listen_table 优化、并发文档
- #1278: IRQ-safe 延迟通知
- #1244: 多接口支持
- #1392: virtio-net 队列序列化修复
- #1319: socket QoS 选项对齐

### 7. 后续文档维护

当新的优化完成后，应同步更新：
1. 更新对应阶段的完成度百分比
2. 将"待推进项"移到"已完成项"
3. 更新代码位置引用
4. 添加新的性能测试基线数据

---

**对应 commit**: 6a857920
