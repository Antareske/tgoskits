# AIC8800 数据面优化 · 阶段交付（2026-09-29）

## 1. 一句话交付物

把「一台能在板上一眼核对闭合的相位计时器」做出来并跑通：它给出的结论是
**总线已跑在原始速率的 96%，TX 之外的时间主要不是准备，而是完成路径空手结束调用、
环里的帧没人取**。据此改写了下阶段的优先级。

## 2. 位置与状态

| 项 | 值 |
| --- | --- |
| 分支 / 工作树 | `sg2002/wifi-opt` / `wt-sg2002-wifi-opt` |
| HEAD | `a6633f3f6`（本阶段未新增提交） |
| 相对 `dev` | 27 个提交（本阶段之前的第三阶段及更早） |
| 未提交 | 14 个项目文件（探针 + 旋钮 + 本轮仪器），`+1501 −151` |
| 不追踪 | `.claude/`、`.ocr/`——**不得纳入提交** |

## 3. 交付内容

### 3.1 测量仪器（本阶段的代码主体）

- **设备层时钟**：平台经 `AicRdifOptions::probe_clock` 把 `axklib::time::monotonic_nanos` 借给驱动，
  与运行时的 `AicInput::now`、与中断戳**同域**（同一个函数）。时钟不存在时探针报**无样本**并印 `clock=off`，
  不报 0。
- **一笔事务的四条腿**：`dispatch`（emit→传输层接手）、`dma`（取缓冲+填+flush）、`program`（描述符+寄存器）、
  `bus`（编程结束→**结束该事务的中断**）。`bus` 的终点由驱动自己的中断处理函数按事件类别打戳
  （只有 CommandComplete/TransferComplete/Error 才算结束事务），不再取"这段时间里最后一次中断"。
- **owner 侧八段**：`release`/`rx_copy`（分开：写只释放、读还要分配并整篇拷出）、`pull`、`teardown`、
  `form`、`bytes`、`rx_parse`、`report`（上报自身开销，记在下一窗口）。
- **`advance` 行**按所取完成的类别归档，**没有取到完成的步归 `other`**；每步都有归属，可与段相减。
- **`split drops`**：完成中断未被看到、因而无法拆分往返的事务数——把"静默为零"变成显式计数。

语义变更（跨轮不可比，已登记）：往返起点从 owner 调用入口改为 **emit 时刻**；`pre/isr/post` 与 `bus`
归到**完成该事务的中断**；每段带一次读时钟的加性偏移（40 ns 分辨率）。

### 3.2 首轮实板读数（`[com COM6] (2026-09-29_065738)`）

配置 `clock=on agg=32x49152`、framed 路径。**纯 TX 与双向分开统计**，各 10 个稳态窗口。

| | 纯 TX（80.769–98.786 s） | 双向（104.789–122.796 s） |
| --- | ---: | ---: |
| 写笔数 / 每笔字节 | 375 / 21063 B | 298 / 24923 B |
| 周期（完成→完成） | 5310 µs | 6459 µs |
| 往返（占周期） | 2458（46%） | 2803（43%） |
| ├ `bus` | 1869 | 2218 |
| ├ `post` | 389 | 372 |
| ├ `dma` | 140 | 154 |
| └ `program` / `dispatch` | 53 / 3 | 55 / 3 |
| `P`(pull+form+bytes) | 379 | 465 |
| 空档 | 2852 | 3655 |
| ├ `advance` 工作 | 362 | 539 |
| ├ `pull` | 105 | 121 |
| ├ credit 退避 | **1017** | 418 |
| └ 其余（停着没跑） | 1354 | 2506 |

仪器自检：`pre` = 四腿之和逐窗闭合（±9 µs / 0.3%）；四条腿样本数 = 写笔数（发送侧零丢样本）；
`写笔数 × 周期 = 96.3% dt`。

### 3.3 三条结论

1. **总线不再是可动项。** `bus` 按写长分桶两项拟合：边际 ≈ **11.2 MB/s**、截距 ≈ **0**，即数据相达总线
   原始速率（11.72 MB/s）的 **96%**；接收读 11.66 B/µs = **99.5%**。第三阶段"每笔固定开销 445~604 µs"
   是旧往返口径（起点在 owner 调用入口）的产物，按 emit 锚点重测**不存在**。
2. **周期的一半以上不是往返。** 空档占周期 54%/57%，其中 credit 退避 1017/418 µs，其余 1354/2506 µs；
   总线占用只有写 35.0%/33.1% 加接收读约 8%。
3. **机制：完成路径空手结束，环里的帧没人取。** 三组读数并排：写完成时 RDIF 环非空 **62.6%/53.9%**；
   `chain write` **0%**、`chain flow` 32%/26%、`chain idle` 49%/53%；代码上核心无事时返回
   `WaitForInterrupt`，owner 对此**立即结束本次调用**（`rdif/owner/progress.rs:401-406`），
   同调用内排在后面的环→核心 `pull`（`submit_one_tx`，`progress.rs:320`）因此不执行 ⇒
   下一笔写要等下一个外部事件 ⇒ 空档 2.9/3.7 ms。`supply` 里 core+ring 占窗口 21–23%，
   而"哪里都没有帧"只占 19–22%。

### 3.4 与并行设计文档的对照

`gpt/aic8800-hardware-async-pipeline-design.md`（该文档未改动）的 §1.2 基础数字**全部复现**
（`P` 379/465、`dma` 140/154、`bus` 1869/2218、`post` 389/372、环非空 62.6%/53.9%、深度≥4 56.4%/49.4%、
到达时队列非空 95.2%/95.8%）。三处修正/补全：

1. 它以**服务周期**（`P`+往返+post = 2843 µs）为分母给出"最多遮蔽 13.3%/18.3%"；实测周期 5310 µs，
   服务周期只占 **54%**，吞吐口径下是 **7.1%/9.8%**。
2. 未列 **credit 退避**（纯 TX 1017 µs/笔，大于它要遮蔽的 `P` 379 µs）。
3. 未用 `chain` 计数器；而它引用的"完成前 RDIF 非空 62.6%"恰是"帧已在环里、只差一次 pull"的证据。

## 4. 验证记录

| 项 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 干净 |
| `cargo xtask clippy` | aic8800 3/3、ax-net 9/9、rdif-eth、ax-driver 51 项，全过 |
| 单测 | `cargo test -p aic8800 --features rdif,host-test`：178 + 1 |
| 工程测试 | `cargo xtask test --since dev` 全过 |
| 板级内核 | `licheerv-nano-sg2002-wifi.toml` 构建通过；FIT 内核 crc32 与产物逐字节一致 |
| 新增判别性单测 | `owner_step_is_filed_against_the_completion_it_took_up`；对"丢掉无完成步"的旧行为验证为**失败** |
| OCR 审查 | round-3，6 名实例（principal ×2、quality ×2、performance、reliability）：**REQUEST CHANGES**，5 blocker 全部修完 |

本轮审查抓到两处会毁掉整轮测量的缺陷，交付前已修：**链式续写的成形原本未被计时**（`Form` 只包在
`drive_ready`，而 `continue_transmit_pipeline` 自己直接调 `prepare_next_transmit`——恰是饱和发送的主路径）；
**完成中断原本没有身份**（旧戳取"最后一次非 spurious 中断"，落在 post 窗口里的卡中断会把宿主唤醒记进 `bus`，
估计影响 10~20% 的完成）。

## 5. 镜像与资产

均在 `C:\Users\Asta\Desktop\build`（WSL `/mnt/c/Users/Asta/Desktop/build`）。

| 镜像 | 内容 | 状态 |
| --- | --- | --- |
| `..._prepa_20260928.img` | A 臂：现状 | 已测 |
| `..._prepdirect_20260928.img` | B 臂：`tx-prep-direct=1` | 已测 |
| `..._nowait_20260928.img` | C 臂：`tx-credit-wait-us=0` | 已测 |
| `..._dmaprobe_20260928.img` | 加 DMA 分配/释放计数 | 已测 |
| `..._attrib_20260928.img` | 上一轮归因探针 | 已测（`owner step` 整场零样本 → 发现探针接错路径） |
| **`..._attrib2_20260928.img`** | **本阶段最终镜像：设备层时钟 + 全归因探针** | **已测（§3.2）** |

基座 `..._q64_20260928.img`（K=32 / 48 KiB / ring 64），DTB `www/sg2002/wifi-sta/lcn-sta-defer0.dtb`
（`tx-prep-direct=0`、credit 等待默认 300 µs）。

## 6. 复现步骤

```bash
# 内核：固件缓存与编译期 STA 凭据都必须给（缺凭据内核能起但不关联，取错 SSID 会因关联被拒而 panic）
AIC8800_FIRMWARE_DIR=<已校验固件缓存> \
STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 \
  cargo xtask starry build -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml
strings target/riscv64gc-unknown-none-elf/release/starryos.bin | grep -c aasta   # 出镜像前先验

# 镜像：在基座上只换内核 + DTB
sg2002-image-build.sh update-kernel <基座.img> \
  --kernel target/riscv64gc-unknown-none-elf/release/starryos.bin \
  --dtb www/sg2002/wifi-sta/lcn-sta-defer0.dtb -o <本轮.img>
```

## 7. 遗留与未完成

1. **未提交**：14 个项目文件。探针（`probe.rs` 的日志行、各调用点的打点、`Stage`/`Leg` 类型、
   `AicRdifOptions::probe_clock`、驱动中断里的完成戳）**开 PR 前必须整块移除**。
2. **两个旋钮默认值未定**：`aic,tx-prep-direct`（默认 0）、`aic,tx-credit-wait-us`（默认 300 µs，
   上界已加为 0..=5000 µs）。
3. **本轮认定的机制尚未实现**（下一阶段首选，见 §8）。
4. **观测缺口**：`TX_CREDIT`（流控读）类没有单独打印，`advance` 与 `irq split` 两行都看不到它，
   而 `chain flow` 占写的 26–32%；`supply` 三类只覆盖空档的约 85%（窗口首个 gap 无前序完成时不计）。
5. **`post`（389 µs/笔）的调度归因未做**——handoff 里记的 `ebpf/sched_trace` 项仍未跑。
6. 并行设计文档的状态机方案（staged/prepared/in-flight 类型分离与提交门控）**未实现**；按 §3.4 建议降级。
7. 与本阶段无关的既有缺陷仍挂着：`device/control.rs` 的 `scan_command` 载荷缓冲越界（tracker 已记）。

## 8. 下一步优先级

1. **首选**：让完成路径在环里有帧时**不要空手结束调用**——返回等待前做一次有界的环→核心 pull，
   或在 owner 侧允许再走一轮 loop。改动在 owner，不动核心分层、不动"单笔在飞"，直接冲那 2.9/3.7 ms 空档。
   判据：`chain idle` 占比下降、`完成→提交` 空档下降，且接收/控制不退化。
2. **次选**：credit 退避（纯 TX 1017 µs/笔）与 `chain flow`（缓存 credit 落到保留位就改排流控读）合并处理。
3. **`post`** 独立做调度归因（与 1、2 互不替代）。
4. **prepare-ahead（逻辑双槽）**：上限 `P`/周期 = 7.1%/9.8%，且证据表明帧已在环里，故排在 1 之后；
   若 1 落地后空档仍大，再按设计文档的状态机方案推进。

## 9. 文档索引

| 文档 | 定位 |
| --- | --- |
| `aic8800-bottleneck-analysis.md` | 当前认知；**§2.29 为本阶段结论**、§2.28 为口径与盲区 |
| `aic8800-async-optimization-plan.md` | 执行方案；**§8 为本阶段（仪器 + 结论 + 优先级）** |
| `aic8800-optimization-tracker.md` | 逐轮「改动 → 测试 → 现象 → 结论」与本阶段实板读数 |
| `aic8800-driver-principles.md` | 驱动分层与运行原理 |
| `gpt/aic8800-hardware-async-pipeline-design.md` | 并行产出的 prepare-ahead 设计；对照见 §3.4 |
| `handoff-20260928.md` | 本阶段开始时的交接文档（其 §9/§10 的部分结论已被本阶段更正） |
| `.ocr/sessions/2026-09-28-sg2002-wifi-opt/rounds/round-3/` | 本阶段 OCR 审查（6 实例、REVIEW、修复清单） |
| 原始板测日志副本 | 证据，置于本目录 |

项目侧（被追踪）的正式文档是 `docs/design/unified-sdio-aic8800.md`：契约与 FDT 参数以它为准。
本阶段未改动任何被追踪的文档。
