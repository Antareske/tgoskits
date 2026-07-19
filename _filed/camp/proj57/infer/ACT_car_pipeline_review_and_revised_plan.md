# ACT 小车推理流水线：方案评审与修订建议（StarryOS 平台）

> 本文是对 `ACT_car_ISP_RGA_NPU_3buffer_pipeline.md` 的系统 + 算法评审，结合 StarryOS / 本仓库（tgoskits）的实际能力，给出修正后的可落地方案。
>
> 核心结论：**原方案方向正确、整体可行，但有 3 个概念性问题和 2 个算法升级点需要修订**，否则落地会踩坑或做无用功。

---

## 0. 一页结论（TL;DR）

| 维度 | 原方案判断 | 修订后结论 |
|------|-----------|-----------|
| 运行模型 | 暗示三线程/事件驱动 | **用户态 C/C++ + pthreads**（StarryOS 用户态无 Rust async runtime） |
| 全程零拷贝 | 作为目标 | **只承诺 RGA→NPU 段零拷贝**；USB 采集段接受一次拷贝 |
| 采集池(ISP/RGA) 三缓存 | 合理 | **合理，保留 3 块** |
| NPU 输入池三缓存 | 合理 | **偏冗余，2 块即可**，甚至可把 RGA 折叠进 NPU 线程 |
| RGA 是否流式 | 未明确 | **不是流式**，是整帧 blit（src→dst 一次性，亚毫秒~几ms） |
| ACT 历史帧窗口 | 需要最近 8 帧 | **大概率是误解**：标准 ACT 是单帧输入，需核实后删除第 11 节 |
| chunk 消费方式 | 开环执行整个 chunk | **升级为 temporal ensembling / 滚动重规划** |

---

## 1. 平台事实：StarryOS 上"应该怎么跑"

调研本仓库后确认的关键事实（决定整个方案形态）：

### 1.1 已验证路径是用户态 C/C++

本仓库已有跑在 OrangePi-5-Plus 上的实测 demo `rknn_yolov8_stream`：

- 用户态标准 Linux ELF 程序
- `libuvc`（USB 摄像头采集）+ `librga.so`（RGA 图像处理）+ `librknnrt.so`（NPU 推理）
- pthreads 做采集/推理并发
- StarryOS 提供 Linux-ABI 兼容：clone/clone3/execve、futex、epoll、io_uring、mmap、ioctl、dlopen 加载 `.so`

> **结论：方案应按"用户态 C/C++ + pthreads"落地，不要按内核 async 设计。** 原文第 8 节"三线程"方向对，只需明确它是 pthreads，不是 async task。

### 1.2 异步支持：分两层，别搞混

| 层 | 异步能力 | 适用场景 |
|----|---------|---------|
| 内核 Rust | **有完整 async 执行器**：`axtask` 的 `block_on`、`register_irq_waker`（IRQ→waker 桥）、`axpoll::PollSet`（见 `os/arceos/modules/axtask/src/future/poll.rs`） | 仅当你要写**内核驱动**时相关 |
| 用户态 | **无 Rust async runtime**；用 pthreads + mutex/condvar + **futex** + **epoll** | 本方案落地层 |

> 本方案运行在用户态，所以**并发就用 pthreads + condvar/futex + epoll**，完成事件用这些原语，不要 busy-wait。

### 1.3 驱动现状：ISP / RGA / V4L2 内核驱动缺失

| 组件 | 内核驱动 | 用户态库 |
|------|---------|---------|
| NPU (Rockchip RKNPU) | ✅ `drivers/npu/rockchip-npu` | `librknnrt.so` |
| TPU (SG2002, 带 ION 分配器) | ✅ `drivers/tpu/sg2002-tpu` | — |
| USB 摄像头 (UVC) | ✅ `drivers/usb/usb-device/uvc`（异步） | `libuvc` |
| **ISP** | ❌ 仅 DTS 引用，无 Rust 驱动 | — |
| **RGA** | ❌ 仅电源域条目，无 Rust 驱动 | `librga.so` |
| **V4L2** | ❌ 无该子系统 | — |

> **影响：纯内核 Rust 流水线需从头写 ISP + RGA + V4L2 三个驱动，不现实。走用户态库路径。**

### 1.4 DMA 基础设施（若将来走内核路径才需要）

`memory/dma-api` 成熟可用：`alloc_coherent`、`alloc_contiguous`、`map_streaming`、`ContiguousBufferPool`、以及显式 `flush()/invalidate()/flush_invalidate()` cache 操作。用户态路径下这些由 librga/RKNN/ION 内部处理。

---

## 2. 零拷贝：能做到哪一段

USB 摄像头路径**无法全程零拷贝**：

```
USB URB buffer ──(libuvc 一次拷贝, 接受)──▶ 用户帧 buffer
用户帧 buffer ──(RGA dma-buf fd import)──▶ RGA dst buffer ──(零拷贝)──▶ NPU
                         └────────── 真正的零拷贝段 ──────────┘
```

- **能零拷贝**：RGA→NPU。RGA 支持 dma-buf fd import；RKNN 有 `rknn_create_mem_from_fd` 直接用 fd 喂输入，不经 CPU 拷贝。
- **不能零拷贝**：USB 采集段。libuvc 几乎必然有一次 URB→用户 buffer 拷贝。
- 想全程零拷贝须上 **MIPI CSI + 硬件 ISP + V4L2 dma-buf**，但这在 StarryOS 上要写 3 个缺失驱动，初版不做。

> **承诺边界：采集一次拷贝 + RGA→NPU 零拷贝。** 这已足够把 CPU 从"逐像素搬运几十万像素"里解放出来。

---

## 3. 三缓存评审：到底要几块

### 3.1 原理：缓存数量是排队论问题

一个生产者-消费者对之间，要让**两端都不阻塞**：
- 1 块 = 强制串行
- 2 块 = 生产者比消费者快时会撞墙（无处可写或覆盖未读数据）
- **3 块 = non-blocking 双向解耦的理论最小值**：一块在写、一块"最新已完成待取"、一块在读

所以"3"不是拍脑袋，是 triple buffering 的标准结论。**但是否需要 3，取决于两端速度差。**

### 3.2 采集池（摄像头/ISP 写 ↔ RGA 读）：保留 3 块 ✅

- 摄像头是连续 USB 流，采集**绝不能停顿**（停顿丢包/掉帧率）。
- 生产者(采集)必须永不阻塞，消费者(RGA)取最新帧。
- 这是教科书 triple buffer 场景。**3 块正确，保留。**

### 3.3 NPU 输入池（RGA 写 ↔ NPU 读）：偏冗余，2 块即可 ⚠️

关键观察：**RGA 极快（亚毫秒~几ms），NPU 是瓶颈（几十ms）。**

当生产者(RGA)远快于消费者(NPU)时，三缓存的第三块永远用不上——RGA 随时能在 NPU 要数据前瞬间产出最新帧。因此：

- **方案 A：2 块 ping-pong**。NPU 推一块，RGA 把下一块备好，latest-only 丢旧。
- **方案 B（更优）：取消独立池，把 RGA 折叠进 NPU 线程**。NPU 每轮推理前：现取最新采集帧 → 同步调一次 RGA → 零拷贝喂 NPU。RGA 那几 ms 相对 NPU 几十 ms 可忽略，没必要为它单开一级流水线 + 三缓存池。

> **结论：两级各 3 块（共 6）确实冗余。推荐"采集池 3 + NPU 输入 2"，或"采集池 3 + RGA 折叠进 NPU 线程（输入退化为 ping-pong 2）"。后者结构最简、延迟最低、状态机最不易写错。**

### 3.4 RGA 是流式处理吗？不是

RGA 是**整帧 blit**：提交一次 src buffer→dst buffer 的 2D 操作（可异步，但粒度是整帧），不是 ISP 那种逐行 streaming，中途无法 tap。

- 所以 dst buffer 省不掉（resize 后尺寸都变了，不可能 in-place）。
- 但因为它"一次性整帧且很快"，**不需要为它做深缓冲**。
- 原文第 7 节末尾"不建议初版依赖 in-place"是对的。

---

## 4. 算法升级（比 buffer 更重要的两点）

### 4.1 ACT 大概率是单帧输入 —— 核实后删掉历史窗口

标准 ACT（Zhao 等人 ALOHA 论文）：
- **输入**：当前**单个时刻**的观测（可多路相机，但同一时刻）+ 本体状态（关节/里程）
- **结构**：CVAE + Transformer
- **输出**：未来 k 步动作 chunk
- **不吃过去 8 帧图像**

原文第 11 节的"history ring 最近 8 帧"很可能是把 ACT 跟视频/时序模型混淆了。

> **影响巨大**：一旦确认 ACT 单帧输入：
> - 第 11 节整节可删
> - "需要连续 8 帧" vs "latest-only 丢帧"的矛盾自动消失
> - 无需保留 8 帧 dma-buf 不释放，NPU 输入池 2 块足矣
>
> **核实方法**（Step 3 看模型 I/O）：单帧 input shape 是 `[1,3,224,224]`；时序模型才会有 `[1,T,3,H,W]`。

### 4.2 chunk 消费方式升级：temporal ensembling / 滚动重规划

原文是"推一次 → 开环执行整个 8 步 chunk → 再推一次"。这是 ACT 论文里**较差的模式**：chunk 边界会抖动，对新视觉不够 reactive。

ACT 论文的杀手锏是 **temporal ensembling**：每个控制步都推理，对**多个重叠 chunk 在同一时刻 t 的预测做指数加权平均**，输出平滑且持续吸收最新观测。代价是要在控制频率上推理——NPU 可能扛不住。

**对 NPU 受限的小车，最佳折中 = 滚动重规划（receding horizon）+ 轻量 ensembling：**

1. NPU 持续以自己最快速度推理（latest-only，这部分原文对了）。
2. **不要等一个 chunk 跑完才用下一个**；保留最近 2~3 个 chunk 的 ring。
3. 对每个控制时刻 t，把**覆盖到 t 的几个 chunk 预测做时间对齐 + 指数加权平均**再下发。

优点：
- 不需要在控制频率上推理（省 NPU）
- 拿到 ensembling 的平滑性和 reactivity
- 严格优于硬切 chunk

实现很轻：把原文第 9 节 action buffer 从"覆盖写"改成"重叠 chunk 的时间对齐加权融合写"。

伪代码：

```c
// NPU 线程：每出一个 chunk，连同其起始时间戳推入 ring
void on_new_chunk(ActionChunk chunk, uint64_t t_start) {
    lock(&ring.lock);
    ring_push(&ring, chunk, t_start);   // 保留最近 N=2~3 个
    unlock(&ring.lock);
}

// 控制线程：每个 tick 融合所有覆盖到 now 的 chunk
Cmd fuse_action(uint64_t now) {
    double w_sum = 0, left = 0, right = 0;
    lock(&ring.lock);
    for (each chunk c in ring) {
        int idx = step_index(now, c.t_start);     // now 落在该 chunk 的第几步
        if (idx < 0 || idx >= CHUNK) continue;     // 该 chunk 不覆盖 now
        double w = exp(-m * age_of(c));            // 越新权重越大
        left  += w * c.left[idx];
        right += w * c.right[idx];
        w_sum += w;
    }
    unlock(&ring.lock);
    if (w_sum == 0) return SAFE_CMD;               // 无覆盖 → 安全兜底
    return clamp_and_limit_accel(left / w_sum, right / w_sum);
}
```

### 4.3 要不要换更先进的模型？

- 对小车 + NPU 实时约束，**ACT 合适，先跑通正确**。
- **不建议 Diffusion Policy**（多步去噪，实时性差）。
- 将来要更强可看 **flow-matching / 一步生成 policy**（一次前向，适合 NPU），但属后话。

---

## 5. 修订后架构

```
USB Camera (UVC)
   │  libuvc 采集  (一次拷贝, 接受)
   ▼
采集帧池 x3   ← triple buffer, 生产者永不阻塞, latest-only
   │  NPU 线程取最新一帧
   ▼
RGA (折叠进 NPU 线程, 同步整帧 blit, dma-buf fd, ~几ms)
   │  零拷贝 fd → RKNN (rknn_create_mem_from_fd)
   ▼
NPU/TPU 跑 ACT (单帧输入!)   ← 瓶颈, 输入 ping-pong x2 足够
   │  输出 action chunk + 起始时间戳
   ▼
Action chunk ring (近 2~3 个 chunk)
   │  时间对齐 + 指数加权融合 (temporal ensembling)
   ▼
独立控制线程 (pthread, 50Hz, condvar/futex)
   │  + 安全兜底 (超时减速/停车/限幅/限加速度)
   ▼
电机
```

---

## 6. 落地要点修订清单

1. **用户态 pthreads 模型**，不用 Rust async（用户态无 runtime）；内核 async 仅在写内核驱动时相关。
2. **采集池 3 + NPU 输入 2**（或把 RGA 折叠进 NPU 线程），砍掉冗余的第二个三缓存。
3. **零拷贝只承诺 RGA→NPU 段**（dma-buf fd + `rknn_create_mem_from_fd`），采集段接受一次拷贝。
4. **核实 ACT 是单帧输入**，确认后删掉原文第 11 节历史窗口，buffer 冲突消失。
5. **cache 一致性**：用户态走 librga/RKNN/ION 已处理；若自管 dma-api buffer，必须 `flush_invalidate`（`memory/dma-api/src/lib.rs` 有接口）。
6. **chunk 消费升级为 temporal ensembling / 滚动重规划**，而非开环执行整 chunk。
7. **锁粒度**：状态机锁只保护"选帧 + 改状态"临界区，硬件执行期间不持锁；完成事件用 condvar/epoll，不 busy-wait。
8. **正在被硬件使用的 buffer 绝不可丢/覆盖**（`*_WRITING` / `*_RUNNING`）——原文第 12 节这点正确，务必严格执行。
9. **时间戳贯穿**：用采集（曝光）时刻作为帧时间戳，一路透传到 action chunk，控制线程据此判过期。

---

## 7. 原方案中保持正确、无需改动的部分

- latest-only 丢旧帧策略（第 12、14 节）——对实时控制完全正确。
- 控制线程独立、不被视觉链路阻塞（第 10 节）。
- 安全兜底：超时减速/停车、限幅、限加速度、断流停车（第 20 节）——专业且必要。
- 渐进式落地 Step 1→8（第 15 节）与"先不追求零拷贝跑通串行版"（第 21 节）——务实，照此推进。
- 时间戳贯穿与状态需加锁/原子（第 18 节）。
- 端到端延迟模型与"防队列堆积比单帧慢更重要"（第 13 节）。
```
