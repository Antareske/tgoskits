# ACT 小车视觉推理流水线方案：ISP/RGA/NPU(TPU) + 三缓存 DMA + 状态机

## 1. 方案目标

本方案用于在开发板小车上运行 ACT（Action Chunking Transformer）或类似视觉控制模型，实现从摄像头输入到左右轮速度输出的低延迟推理流水线。

目标不是“每一帧都完整处理”，而是：

> **永远尽量使用最新视觉数据，持续更新未来若干步控制输出。**

核心设计目标：

1. 摄像头图像经过 ISP 得到稳定可用的画面。
2. RGA 对图像进行硬件级 resize / crop / format 转换。
3. NPU/TPU 执行 ACT 模型推理。
4. DMA buffer 尽量避免 CPU memcpy。
5. 使用三缓存与状态机管理图像帧生命周期。
6. 输出结果区持续更新未来若干步左右轮速度。
7. 控制线程始终读取最新可用控制结果，不等待完整视频帧队列。

---

## 2. 基本概念

### 2.1 ISP 是什么

ISP，即 Image Signal Processor，图像信号处理器。

摄像头 sensor 原始输出通常不是正常图片，而是 RAW/Bayer 数据。ISP 负责把它变成正常图像。

ISP 主要做：

- 去马赛克：RAW/Bayer → RGB/YUV
- 自动曝光 AE
- 自动白平衡 AWB
- 降噪
- 锐化
- 色彩校正
- 可能的畸变校正

在本方案中，ISP 的职责是：

> **把摄像头原始图像变成稳定、干净、可供后续处理的图像帧。**

ISP 不负责理解 ACT 模型，也不直接输出模型 tensor。

---

### 2.2 RGA 是什么

RGA 可以理解为图像搬运与变换硬件，类似一个专用的 2D 图像处理加速器。

RGA 常见能力：

- resize：例如 1920×1080 → 224×224
- crop：裁剪图像区域
- rotate / flip：旋转、翻转
- format convert：例如 NV12 → RGB888
- 图像 buffer 间搬运

在本方案中，RGA 的职责是：

> **把 ISP 输出的图像变成 NPU/TPU 模型更容易接收的尺寸和格式。**

例如：

```text
ISP 输出：1280×720 NV12
RGA 输出：224×224 RGB / BGR / NV12
```

---

### 2.3 NPU/TPU 是什么

NPU/TPU 是神经网络推理硬件。

在本方案中，NPU/TPU 的职责是：

> **读取已经预处理好的图像输入，执行 ACT 模型，输出未来若干步动作。**

例如 ACT 模型输出：

```text
未来 8 步左右轮速度：
[
  (left_0, right_0),
  (left_1, right_1),
  ...
  (left_7, right_7)
]
```

---

### 2.4 DMA 与零拷贝

DMA 不是共享内存本身，也不是“就地计算”。

DMA 是：

> **让硬件设备直接访问内存，避免 CPU 参与数据拷贝的机制。**

零拷贝在这里主要指：

> **CPU 不再把图像从 ISP buffer memcpy 到 RGA buffer，再 memcpy 到 NPU buffer。**

理想链路是：

```text
ISP → DMA buffer → RGA → DMA buffer → NPU/TPU
```

CPU 只负责：

- 分配或管理 buffer
- 提交任务
- 修改状态
- 等待或查询完成事件
- 处理最终控制输出

CPU 不应该逐像素搬运图像数据。

---

## 3. 整体流水线结构

推荐的高层结构如下：

```text
Camera Sensor
    ↓
ISP
    ↓
ISP 输出 DMA buffer
    ↓
RGA resize / crop / format convert
    ↓
NPU/TPU 输入 DMA buffer
    ↓
ACT 模型推理
    ↓
未来 N 步左右轮速度输出区
    ↓
小车控制线程按时间逐步消费
```

更工程化的结构：

```text
┌────────────┐
│ Camera/ISP │
└─────┬──────┘
      │ ISP_DONE frame
      ↓
┌────────────┐
│ Frame Pool │  三缓存 DMA buffer + 状态机
└─────┬──────┘
      │ latest ready frame
      ↓
┌────────────┐
│    RGA     │  resize/crop/format
└─────┬──────┘
      │ RGA_DONE tensor-like image
      ↓
┌────────────┐
│  NPU/TPU   │  ACT inference
└─────┬──────┘
      │ action chunk
      ↓
┌────────────┐
│ Action Buf │  持续更新未来若干步控制结果
└─────┬──────┘
      ↓
┌────────────┐
│ Motor Ctrl │
└────────────┘
```

---

## 4. 三缓存模型

### 4.1 为什么需要三缓存

在视觉推理流水线中，至少有多个硬件阶段可能同时工作：

- ISP 正在写入一帧
- RGA 正在处理上一帧
- NPU/TPU 正在推理更早的一帧

如果只有一个 buffer，所有阶段必然串行。

如果只有两个 buffer，慢阶段容易导致写入端无处可写，或者覆盖未处理数据。

三缓存的意义是：

> **用有限 buffer 让 ISP、RGA、NPU/TPU 在时间上错开工作。**

典型状态：

```text
Buffer A：NPU/TPU 正在读
Buffer B：RGA 正在处理
Buffer C：ISP 正在写
```

或者：

```text
Buffer A：空闲
Buffer B：ISP 写入完成，等待 RGA
Buffer C：RGA 处理完成，等待 NPU/TPU
```

---

### 4.2 三缓存不是严格队列

对小车控制来说，旧帧价值很低。

所以不推荐设计成：

```text
A → B → C → A 严格顺序处理每一帧
```

更推荐设计成：

> **latest-only：永远优先处理最新可用帧，必要时丢弃旧帧。**

原因：

- 小车需要实时性，不需要完整视频流。
- 100ms 前的图像可能已经过时。
- 低延迟比高帧完整率更重要。

---

## 5. Buffer 状态机设计

每个图像 buffer 应该有明确状态，而不是只用“空/满”。

推荐状态：

```c
enum BufferState {
    FREE,           // 空闲，可被 ISP 写入
    ISP_WRITING,    // ISP 正在写
    ISP_DONE,       // ISP 写完，等待 RGA
    RGA_RUNNING,    // RGA 正在处理
    RGA_DONE,       // RGA 处理完，等待 NPU/TPU
    NPU_RUNNING,    // NPU/TPU 正在读取并推理
};
```

状态流转：

```text
FREE
  ↓
ISP_WRITING
  ↓
ISP_DONE
  ↓
RGA_RUNNING
  ↓
RGA_DONE
  ↓
NPU_RUNNING
  ↓
FREE
```

对于 latest-only 策略，可以允许旧的 `ISP_DONE` 或旧的 `RGA_DONE` 被丢弃：

```text
旧 ISP_DONE，如果还没被 RGA 取走，可直接标记 FREE
旧 RGA_DONE，如果还没被 NPU 取走，可直接标记 FREE
```

但注意：

> 正在被硬件使用的 buffer 不能覆盖。

即：

- `ISP_WRITING` 不能被别人读写
- `RGA_RUNNING` 不能被 ISP 覆盖
- `NPU_RUNNING` 不能被释放或复用，直到 NPU/TPU 完成

---

## 6. 两级三缓存更清晰

实际工程中，建议把 buffer 分成两级，而不是把 ISP/RGA/NPU 全挤在同一个池里。

### 6.1 ISP 输出池

用于保存摄像头/ISP 输出的原始图像帧：

```text
ISP Pool:
  isp_buf[0]
  isp_buf[1]
  isp_buf[2]
```

状态：

```text
FREE → ISP_WRITING → ISP_DONE → RGA_READING → FREE
```

### 6.2 RGA 输出池 / NPU 输入池

用于保存已经 resize / format 转换后的模型输入图像：

```text
NPU Input Pool:
  npu_buf[0]
  npu_buf[1]
  npu_buf[2]
```

状态：

```text
FREE → RGA_WRITING → RGA_DONE → NPU_RUNNING → FREE
```

推荐结构：

```text
Camera/ISP
   ↓
[ISP 三缓存池]
   ↓
RGA
   ↓
[NPU 输入三缓存池]
   ↓
NPU/TPU
```

这样比“所有模块共用三个槽”更容易落地，也更容易避免状态混乱。

---

## 7. 为什么建议两级三缓存

如果 ISP、RGA、NPU 共用同一个三缓存池，需要非常精细地管理每个 buffer 同时作为输入/输出的状态，容易写错。

两级 buffer 更清楚：

- ISP pool 管摄像头原始帧
- NPU input pool 管模型输入帧

RGA 做的是：

```text
从 ISP pool 取一个最新 ISP_DONE buffer
向 NPU input pool 写一个 FREE buffer
```

这符合 RGA 的真实行为：

> **RGA 通常是读一个 src buffer，写一个 dst buffer。**

而不是简单“在同一个 buffer 里原地变换”。

虽然某些硬件支持部分 in-place 操作，但不建议初版依赖 in-place。

---

## 8. 三线程/三任务结构

可以将程序拆成三个主要任务：

### 8.1 ISP/Capture 任务

职责：

- 从摄像头取帧
- 等 ISP 完成
- 把 buffer 标记为 `ISP_DONE`
- 如果池中没有 FREE buffer，则丢弃最旧未处理帧，保证新帧优先

伪代码：

```c
while (running) {
    buf = get_free_or_drop_oldest_isp_buffer();
    mark(buf, ISP_WRITING);

    camera_capture_to_dma_buffer(buf);

    buf.timestamp = now();
    mark(buf, ISP_DONE);
}
```

---

### 8.2 RGA 任务

职责：

- 从 ISP pool 中取最新 `ISP_DONE` buffer
- 从 NPU input pool 中取一个 `FREE` buffer
- 提交 RGA resize / crop / format 转换
- RGA 完成后，把输出 buffer 标记为 `RGA_DONE`
- 输入 ISP buffer 释放为 `FREE`

伪代码：

```c
while (running) {
    src = get_latest_isp_done_buffer();
    dst = get_free_or_drop_oldest_npu_buffer();

    if (!src || !dst) {
        sleep_or_wait_event();
        continue;
    }

    mark(src, RGA_RUNNING);
    mark(dst, RGA_WRITING);

    submit_rga(src, dst);
    wait_rga_done();

    mark(src, FREE);
    dst.timestamp = src.timestamp;
    mark(dst, RGA_DONE);
}
```

---

### 8.3 NPU/TPU 任务

职责：

- 从 NPU input pool 取最新 `RGA_DONE` buffer
- 提交 ACT 模型推理
- 获得 action chunk
- 更新共享动作输出区
- 释放输入 buffer

伪代码：

```c
while (running) {
    input = get_latest_rga_done_buffer();

    if (!input) {
        sleep_or_wait_event();
        continue;
    }

    mark(input, NPU_RUNNING);

    action_chunk = run_act_model(input);

    update_action_buffer(action_chunk, input.timestamp);

    mark(input, FREE);
}
```

---

## 9. ACT 输出结果区设计

ACT 模型通常不是只输出当前一步动作，而是输出未来若干步动作。

例如：

```text
未来 8 步动作：
t+0: left_0, right_0
t+1: left_1, right_1
...
t+7: left_7, right_7
```

建议设置一个独立的共享动作区：

```c
struct ActionChunk {
    uint64_t frame_timestamp;
    uint64_t update_timestamp;
    int valid;
    float left_speed[8];
    float right_speed[8];
};
```

NPU/TPU 每次推理完成后，直接覆盖更新这个动作区。

控制线程不等待 NPU，而是按固定频率读取最新动作区。

---

## 10. 控制线程设计

控制线程应该独立于视觉推理线程。

例如控制频率 50Hz：

```c
while (running) {
    action = read_latest_action_chunk();

    if (action.valid && !expired(action)) {
        cmd = pick_action_by_time(action);
        send_motor_command(cmd.left, cmd.right);
    } else {
        send_safe_command();
    }

    sleep_until_next_control_tick();
}
```

关键原则：

> **控制线程不能被摄像头、RGA 或 NPU 阻塞。**

如果视觉推理暂时慢了，控制线程可以：

1. 继续使用上一轮 action chunk 的后续动作。
2. 如果 action chunk 过期，则减速或停车。
3. 如果连续多次无新输出，则进入安全模式。

---

## 11. ACT 历史帧窗口

如果 ACT 模型需要历史帧，例如最近 8 帧图像，则需要额外维护一个 history ring。

### 11.1 简单版本

NPU 每次拿到最新 RGA_DONE 后，把它加入历史窗口：

```text
history[0..7] = 最近 8 个预处理后图像
```

然后用这 8 帧作为模型输入。

### 11.2 低延迟版本

不强行等待凑齐连续 8 帧，而是：

- 启动阶段不足 8 帧时重复最近帧或填充默认帧。
- 稳定阶段只保留最新 8 个时间戳。
- 如果 RGA/NPU 丢帧，历史窗口允许时间间隔不完全一致。

原则：

> **历史窗口服务于控制实时性，不服务于视频完整性。**

---

## 12. latest-only 丢帧策略

建议规则：

### 12.1 ISP pool 丢帧

当 ISP 要写入新帧，但没有 FREE buffer：

1. 优先丢弃最旧的 `ISP_DONE`。
2. 如果没有 `ISP_DONE`，说明 RGA 正在处理、ISP 正在写，没有安全可丢帧。
3. 此时可以：
   - 跳过本次采集；
   - 或等待很短时间；
   - 或降低摄像头帧率。

不能覆盖：

```text
ISP_WRITING
RGA_RUNNING
```

### 12.2 NPU input pool 丢帧

当 RGA 要写输出，但没有 FREE buffer：

1. 优先丢弃最旧的 `RGA_DONE`。
2. 如果没有 `RGA_DONE`，说明 NPU 正在跑、RGA 正在写。
3. 此时 RGA 可以短等，或丢弃本次输入帧。

不能覆盖：

```text
RGA_WRITING
NPU_RUNNING
```

### 12.3 NPU 取帧

NPU 永远取最新 `RGA_DONE`。

旧的 `RGA_DONE` 可以直接释放。

---

## 13. 延迟模型

端到端延迟近似为：

```text
T_total =
    T_exposure
  + T_isp
  + T_wait_rga
  + T_rga
  + T_wait_npu
  + T_npu
  + T_control_tick
```

其中：

- `T_exposure`：摄像头曝光时间
- `T_isp`：ISP 处理时间
- `T_rga`：resize / format 转换时间
- `T_npu`：模型推理时间
- `T_control_tick`：控制线程周期等待
- `T_wait_rga` / `T_wait_npu`：由调度与 buffer 造成的等待

优化目标不是让每项为 0，而是：

> **避免队列堆积，让等待项尽量不随时间增长。**

最危险的不是单帧慢，而是队列越积越长。

所以必须使用 latest-only 策略。

---

## 14. 为什么不建议普通 FIFO 队列

普通 FIFO 会保证每帧都处理：

```text
Frame 1 → Frame 2 → Frame 3 → Frame 4 → ...
```

但小车控制不适合这样。

如果 RGA 或 NPU 变慢，FIFO 会堆积：

```text
当前正在处理 200ms 前的帧
```

这对控制是危险的。

因此推荐：

```text
只处理最新帧，旧帧直接丢弃
```

即：

```text
Frame 1 → 丢
Frame 2 → 丢
Frame 3 → 处理
Frame 4 → 丢
Frame 5 → 处理
```

---

## 15. 推荐落地步骤

### Step 1：先跑通摄像头

目标：

- 能从摄像头取到图像帧
- 确认分辨率、格式、帧率
- 确认 ISP 是否正常工作

优先使用：

- 官方 camera demo
- V4L2 demo
- SDK 示例程序

需要记录：

```text
输出格式：NV12 / RGB / YUV
输出尺寸：例如 1280×720
帧率：例如 30 FPS
```

---

### Step 2：单独跑通 RGA

目标：

- 输入一张图
- RGA resize 到模型输入尺寸
- RGA 输出格式符合 NPU/TPU 模型输入要求

需要确认：

```text
模型输入尺寸：例如 224×224
模型输入格式：RGB / BGR / NV12
模型输入数据类型：uint8 / int8 / fp16 / fp32
```

---

### Step 3：单独跑通 NPU/TPU

目标：

- 用静态图片或固定输入跑模型
- 确认 ACT 模型可执行
- 确认输出左右轮速度格式

需要确认：

```text
输入 shape
输入 dtype
输入 layout：NHWC / NCHW
输出 shape
量化参数
```

---

### Step 4：串行打通

先不要急着做三线程。

先做最简单串行：

```text
取一帧 → RGA → NPU → 打印输出
```

确认结果正确。

---

### Step 5：加入双 buffer

把 ISP 与 RGA/NPU 稍微解耦，确认不会频繁崩溃。

---

### Step 6：加入两级三缓存

正式改成：

```text
ISP 三缓存池
RGA/NPU 输入三缓存池
```

加入状态机。

---

### Step 7：加入 latest-only

丢弃旧帧，测端到端延迟。

---

### Step 8：加入 ACT 输出 action chunk

让控制线程独立运行，不等待推理线程。

---

## 16. 建议的文件结构

```text
act_car_pipeline/
├── main.c
├── camera/
│   ├── camera_v4l2.c
│   └── camera_v4l2.h
├── rga/
│   ├── rga_preprocess.c
│   └── rga_preprocess.h
├── npu/
│   ├── act_infer.c
│   └── act_infer.h
├── buffer/
│   ├── dma_buffer_pool.c
│   ├── dma_buffer_pool.h
│   ├── buffer_state.c
│   └── buffer_state.h
├── control/
│   ├── motor_control.c
│   └── motor_control.h
└── common/
    ├── timestamp.h
    ├── log.h
    └── config.h
```

---

## 17. 配置参数建议

建议集中放在 `config.h`：

```c
#define ISP_POOL_SIZE 3
#define NPU_POOL_SIZE 3

#define CAMERA_WIDTH 1280
#define CAMERA_HEIGHT 720
#define CAMERA_FORMAT NV12

#define MODEL_WIDTH 224
#define MODEL_HEIGHT 224
#define MODEL_FORMAT RGB888

#define ACT_CHUNK_SIZE 8
#define CONTROL_HZ 50

#define ACTION_TIMEOUT_MS 200
#define MAX_FRAME_AGE_MS 150
```

---

## 18. 状态机注意事项

### 18.1 状态修改必须加锁或原子化

多线程情况下，状态修改需要：

- mutex
- spinlock
- atomic
- 或单线程 event loop

不要让两个线程同时修改同一个 buffer 状态。

---

### 18.2 timestamp 非常重要

每个 buffer 必须有：

```c
uint64_t frame_timestamp;
uint64_t state_update_timestamp;
```

这样可以判断：

- 哪一帧最新
- 当前处理的是不是过期帧
- 哪个阶段耗时最大

---

### 18.3 不要依赖 buffer 编号顺序

不要假设：

```text
A 后面一定是 B
B 后面一定是 C
```

应该按状态和 timestamp 选择。

---

## 19. 输出动作区注意事项

动作区也需要状态保护。

推荐：

```c
struct SharedAction {
    pthread_mutex_t lock;
    uint64_t version;
    uint64_t update_timestamp;
    float left[ACT_CHUNK_SIZE];
    float right[ACT_CHUNK_SIZE];
};
```

NPU 线程更新时：

```text
lock
version++
写入 left/right
update_timestamp = now
unlock
```

控制线程读取时：

```text
lock
复制一份 action chunk 到本地
unlock
```

不要让控制线程直接长期持有共享 action buffer。

---

## 20. 安全策略

小车控制必须有安全兜底。

建议规则：

1. 如果 ACT 输出超过 `ACTION_TIMEOUT_MS` 没更新，逐渐减速。
2. 如果超过更长时间仍无输出，停车。
3. 如果输出速度异常大，限幅。
4. 如果左右轮速度跳变过大，限加速度。
5. 如果摄像头断流，停车。
6. 如果 NPU/TPU 连续失败，停车。
7. 如果帧时间戳过旧，拒绝使用该输出。

示例：

```c
left = clamp(left, -MAX_SPEED, MAX_SPEED);
right = clamp(right, -MAX_SPEED, MAX_SPEED);

left = limit_accel(prev_left, left, MAX_ACCEL);
right = limit_accel(prev_right, right, MAX_ACCEL);
```

---

## 21. 最小可行版本

第一版不追求完美零拷贝，可以先跑通：

```text
V4L2 取帧
→ CPU 可见 buffer
→ RGA resize
→ NPU 推理
→ 打印左右轮速度
```

第二版再改成：

```text
V4L2 DMA-BUF
→ RGA import fd
→ NPU import / set input
```

第三版再做：

```text
两级三缓存
→ latest-only
→ action chunk
→ 控制线程
```

不要一开始就把所有复杂机制一次写完。

---

## 22. 最终推荐架构

最终建议架构如下：

```text
                 ┌────────────────────┐
                 │ Camera Sensor       │
                 └─────────┬──────────┘
                           ↓
                 ┌────────────────────┐
                 │ ISP                │
                 │ RAW → NV12/RGB      │
                 └─────────┬──────────┘
                           ↓
              ┌──────────────────────────┐
              │ ISP DMA Buffer Pool x3    │
              │ FREE/WRITING/DONE/RGA     │
              └─────────┬────────────────┘
                        latest ISP_DONE
                           ↓
                 ┌────────────────────┐
                 │ RGA                │
                 │ resize/crop/format  │
                 └─────────┬──────────┘
                           ↓
              ┌──────────────────────────┐
              │ NPU Input Buffer Pool x3  │
              │ FREE/RGA_DONE/NPU_RUNNING │
              └─────────┬────────────────┘
                        latest RGA_DONE
                           ↓
                 ┌────────────────────┐
                 │ NPU/TPU ACT Model   │
                 └─────────┬──────────┘
                           ↓
              ┌──────────────────────────┐
              │ Shared Action Chunk       │
              │ future 8 wheel speeds     │
              └─────────┬────────────────┘
                           ↓
                 ┌────────────────────┐
                 │ Motor Control Loop  │
                 │ 50Hz / 100Hz        │
                 └────────────────────┘
```

---

## 23. 方案核心总结

本方案的核心不是“让所有帧都被处理”，而是：

> **让小车始终基于最新可用视觉信息做控制。**

最重要的设计原则：

1. ISP 负责图像质量。
2. RGA 负责尺寸和格式。
3. NPU/TPU 负责 ACT 推理。
4. DMA buffer 用来减少 CPU 拷贝。
5. 三缓存用来解耦硬件阶段。
6. 状态机防止 buffer 被错误覆盖。
7. latest-only 策略防止队列延迟累积。
8. ACT 输出区持续覆盖更新。
9. 控制线程独立运行，不能等待视觉线程。
10. 安全策略必须兜底停车。

最终目标：

```text
摄像头持续采集
RGA持续预处理
NPU/TPU持续推理
控制线程持续读取最新动作
旧帧被丢弃
新动作持续覆盖
小车低延迟响应
```
