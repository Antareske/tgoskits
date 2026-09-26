# VendorSetup 响应形状失配：板测 panic 真因记录

- 日期：2026-08-28
- 分支：`sg2002/wifi-irq`（工作区未提交改动，HEAD == dev == 7c5bbd133）
- 板测材料：`www/logs/5.log`、`www/logs/6.log`（均为初始化时立即 panic）
- 关联文档：`www/newdev/改动审查与wifi-panic根因报告.md`（其 §4 根因推断被本文证据修正）

---

## 0. 摘要

板测 5.log/6.log 的 panic 真因是 **VendorSetup 阶段的响应形状失配**：设备核心期待 `Unit`，而 rdif 适配层对 Direct 操作一律映射 `Byte`，`expect_unit(Byte)` 确定性返回 `MalformedResponse`。该失配在 dev 上已存在（既有缺陷，非本次工作区改动引入），板测实测与宿主机单元测试双重实证。此前报告的"邮箱坏帧"根因推断（4.log 失败于 ReadRevision 坏帧）不成立——三个日志的失败点都在 VendorSetup，从未到达邮箱。

---

## 1. 现象

### 1.1 板测日志（5.log / 6.log）

| 项 | 5.log | 6.log |
|---|---|---|
| 拍摄时间 | 16:33 | 16:51 |
| 对应镜像 | check 构建（kernel crc `19ab5acd`；check-kernel.bin 产物 mtime 为 16:34，晚于拍摄 1 分钟，应为拷贝/时钟偏差） | final 构建（kernel crc `46ab89c5`） |
| console active | 6.671862 | 5.422747 |
| panic 时刻 | ~6.711（约 +40ms） | ~5.460（约 +37ms） |
| panic 文本 | `failed to initialize network queue runtime: network queue initialization failed: Other error: AIC core failed: AIC mailbox response was malformed` | 同左 |

两次板测 panic 时刻与文本完全一致（确定性失败），且均发生在 SD 卡初始化进行中（5.log 在 CSD 阶段、6.log 在 OCR 阶段被 panic 打断）。

### 1.2 日志缺失现象

5.log/6.log 中 **executor 任务的所有日志系统性缺失**（`aic:` 前缀 0 行、`sdio: init state` 0 行、`network queue group startup failed` error 级 0 行），而 SD 卡侧（0:9/0:10 任务）的日志正常出现。该现象与"板上跑的是新内核"并存，属板测串口捕获环境问题（代码侧日志量策略是加重因素，见修复记录），不是代码逻辑问题——板测前需解决捕获方式，否则诊断日志失去作用。

4.log 需区分两类缺失：其内核**不含** `aic:` 前缀日志（诊断日志在其后加入），"aic: 0 行"是平凡事实；但其内核**含** executor 的 error 日志（panic 文本带完整错误链证明 QueueInit 改动在场），该日志同样 0 行——后者才是与 5/6.log 共同构成"捕获问题"的证据。

### 1.3 与 4.log 的关系

- 4.log（183ms panic）与 5/6.log（37-40ms panic）的**内核均含 init.rs 循环修复**：4.log 的 panic 文本为 `QueueInit(NetError)` 新格式（与循环修复同属一个未提交 diff），且 dev 无循环修复时寄存器步行为是 60s stall（dev-wifi.log/1.log 的 67s panic 实证）——4.log 在 183ms 收到 VendorSetup 完成值，机制上要求循环修复在场。183→37ms 的差异归因 4.log 全系统偏慢（CSD 读 146ms，报告 §6.4 调度/捕获解释），**该差异在含修复前提下暂无代码解释**，不再作为修复生效证据。
- 循环修复生效的直接证据改为：**5/6.log 在 37ms 到达 VendorSetup，而 dev 无修复必 60s stall**。

---

## 2. 证据链

### 2.1 板上跑的是含全部修复的新内核

- 6.log 的 FIT kernel hash `46ab89c5` 与 `final-kernel.bin`（16:46 构建）的 crc32 字节级一致；5.log 的 `19ab5acd` 与 `check-kernel.bin` 一致。
- 两产物 strings 均含全部新诊断字符串（`aic: mailbox drain`、`aic: mailbox frame did not validate; retrying:` 等）。

### 2.2 失配点（代码事实）

| 层 | 位置 | 行为 |
|---|---|---|
| 核心 | `drivers/net/aic8800/src/device/startup/mod.rs:211-214` | `VendorSetup(index)` 分支 `expect_unit(response)?` —— 期待 Unit |
| 请求构造 | `drivers/net/aic8800/src/device/request.rs:37-43` | `write_byte(...)` → `SdioRequestKind::WriteByte { read_after_write: true }` |
| 适配层 | `drivers/net/aic8800/src/rdif/owner/operation.rs:72-80, 162` | `WriteByte` → `ActiveOperation::Direct`；Direct 完成**一律**映射 `SdioResponse::Byte` |
| 判定 | `drivers/net/aic8800/src/device/request.rs:9-14` | `expect_unit` 对 `Byte` 返回 `Err(MalformedResponse)` |

VendorSetup 三步均为 Direct 写（fn0 0xF2=0x7F、byte_mode 0x07=1、wakeup 0x02=0x11，见 `device/startup/vendor.rs`）。第一步写完成 → 核心收到 Byte → `MalformedResponse` → `fail()` → executor 启动失败 → `devices.rs:97` panic。`git show HEAD` 确认该失配在 dev 上同样存在——**既有缺陷，非本次改动引入**。

### 2.3 单元测试实证

临时测试驱动纯核心（`AicDevice`）至 VendorSetup 完成，喂 `SdioResponse::Byte(0x7f)`：

```
test vendor_setup_byte_completion_fails_the_core ... FAILED
VendorSetup failed with Byte completion: MalformedResponse
```

5ms 模拟时间内在宿主机确定性复现板测 panic 错误。测试为临时验证文件，验证后已删除（该测试可直接作为回归测试雏形）。

### 2.4 时间线吻合

| 阶段 | 6.log 时间预算 |
|---|---|
| executor 派发（init_net） | ~5.423 |
| wifi 卡枚举 | ~30ms（推断值，executor 日志未捕获、无法直接验证） |
| validate → start → EnableFunction/SetBlockSize/EnableFunctionInterrupt | ~1-2ms |
| **VendorSetup(0) 写 fn0 0xF2 → 完成 Byte → 判死** | **~5.456-5.460** |

与 panic 时刻 5.460164 吻合。4.log 的 183ms 差异见 §1.3（含修复前提下暂无代码解释，不用于证明循环修复）。

---

## 3. 对既有根因推断的修正

`www/newdev/改动审查与wifi-panic根因报告.md` §4 推断 4.log 失败于 ReadRevision 邮箱的坏帧（`frame[4..6] != 0x0401`），并据此设计坏帧重试与排空方案。本文证据表明：

- 4.log/5.log/6.log 的 `MalformedResponse` 均出自 VendorSetup 的 `expect_unit` 失配，**从未到达 ReadRevision 邮箱**。
- 报告 §4.2 的排除理由"expect_* 类型不匹配（此处不可能，Direct 操作恒返回 Byte）"恰好漏判——"Direct 恒返回 Byte"正是与"VendorSetup 期待 Unit"的失配点。
- 报告 §4.5 的诚实边界（坏帧字节未实测）因此未能兜住该推断。

坏帧重试与排空仍有价值：修掉 VendorSetup 失配后，邮箱阶段将面对 bootrom 残留帧（旧板测驱动与厂商驱动的既有行为），届时它们正是所需加固。但其定位应从"当前 panic 的解药"修正为"邮箱阶段的预防性加固"。

---

## 4. 修复方向

1. **失配修复（二选一，保持形状契约一致；失配点共 4 处，须全覆盖）**：
   - 核心侧：`consume_startup_response` 的 `VendorSetup`、`Reinitialize`、`ArmChipInterrupt`（startup/mod.rs:211-214, 231-238）与 `consume_completion` 的 `Shutdown`（progress.rs:177-182）分支改为接受 `Byte`（读回值丢弃或校验）；或
   - 适配层侧：rdif 对 `WriteByte` 完成映射 `Unit`（丢弃读回值）。
   - **注意**：只修 `VendorSetup`/`Reinitialize` 两处会把 panic 确定性转移到 `ArmChipInterrupt`（固件上传 + StackStart 邮箱之后）；`Shutdown` 失配使设备永远无法干净停止。
2. **测试先行（项目规范）**：以 2.3 的临时测试为雏形补回归测试——旧实现上必然失败（已实证），修复后应断言推进至下一阶段而非 `Failed`；4 个失配点各一条（阶段注入定向测试，每条 5-10 行）。
3. **板测捕获环境**：executor 日志在现有串口捕获中系统性丢失，需先解决（波特率/捕获工具），否则下一轮板测仍是盲盒。
4. 修复后预期：panic 点推进到 ReadRevision 邮箱；若再失败，5s 超时 + 帧头 warn（坏帧重试路径）与 `aic:` 诊断日志将给出可定位证据。

---

## 附录：关键位置索引

- `drivers/net/aic8800/src/device/startup/mod.rs:198-224` — `consume_startup_response`（VendorSetup 判死点 :211-214）
- `drivers/net/aic8800/src/device/startup/vendor.rs:9-24` — V3 vendor 写入序列
- `drivers/net/aic8800/src/device/request.rs:10-27, 37-43` — `expect_unit` 判定与 `write_byte` 构造
- `drivers/net/aic8800/src/rdif/owner/operation.rs:68-80, 158-162` — WriteByte → Direct → Byte 映射
- `drivers/net/aic8800/src/rdif/owner/progress.rs:325-345` — `validate_card_identity`（init 完成后的关卡）
- `drivers/net/aic8800/src/rdif/error.rs:12` — `AIC core failed: {0}` 包装（panic 文本来源）
- `net/ax-net/src/queue_runtime/executor/mod.rs:198-222` — initialize 循环（失败 error 日志 :217）
- `os/arceos/modules/axruntime/src/devices.rs:97` — panic 点

---

## 修复记录（2026-08-28 晚，按审查裁决方案 (a) 实施）

第二轮审查（5 位 reviewer）裁决方案 (a)（核心侧接受 Byte），并按测试先行落地。改动集中在 `drivers/net/aic8800/src/device/`、`drivers/blk/sdmmc-protocol/`、`net/ax-net/`：

1. **失配修复（4 点全覆盖）**：`request.rs` 新增 `expect_write_ack`（接受 Byte、丢弃读回值，注释说明 R5 读回字节语义）；`consume_startup_response` 的 `VendorSetup`/`Reinitialize`/`ArmChipInterrupt` 与 `consume_completion` 的 `Shutdown` 四个消费点统一改用；`ArmChipInterrupt` 修复后启动流程可到达 `Complete` → `Event(Started)`。
2. **契约文档化**：`request.rs` 顶部固化"请求 kind → 完成形状"对偶契约表（含失配事故说明）；`SdioRequestKind` doc 注释交叉引用；`write_byte` 补 `read_after_write`（RAW 位）语义注释。
3. **错误措辞**：`MalformedResponse` Display 从 "AIC mailbox response was malformed"（邮箱专属措辞，正是把根因分析引向邮箱误诊的文本）改为形状中立的 "AIC SDIO completion did not match the expected response shape"。
4. **Ready 态排空限定**（round 1 Blocker 1）：邮箱初始相位按状态决定——`Starting` 才 `DrainCount` 排空（bootrom 残留仅初始化期存在），`Ready` 态直接 `Flow`，不再与 RX 路径竞争 FIFO。
5. **日志级别**：`aic: submit sdio`/`aic: irq snapshot` 降 debug（数据面热路径）；`sdio: init state` 降 debug（落实文档计划）；`aic: mailbox count raw=` 暂留 info（下一轮板测的邮箱诊断依赖），**降级计划**：邮箱阶段板测验证完成即降 debug（与 flow 无信用路径对齐）。
6. **收尾**：`startup_error` 的 `unwrap_or(NotSupported)` 改为带注释 `expect`（不可达不变量）；坏帧帧头切分两处裸 `unwrap` 改带上下文 `expect`；drain rounds 记账不变式、死臂哨兵、executor 注释（补 devices.rs:97 位置）、两个 begin 函数日志风格统一等注释收尾。

**测试先行（6 条新测试，旧实现上全部验证必然失败）**：

| 测试 | 位置 | 行为契约 |
|---|---|---|
| `vendor_setup_accepts_the_write_read_back_byte` | device/progress.rs tests | 喂 Byte → 推进 VendorSetup(1) 而非 Failed |
| `reinitialize_accepts_the_write_read_back_byte` | 同上 | 同上（阶段注入） |
| `arm_chip_interrupt_accepts_the_write_read_back_byte` | 同上 | 喂 Byte → `Event(Started)` + Ready |
| `shutdown_accepts_the_write_read_back_byte` | 同上 | 喂 Byte → `Event(Stopped)` + Stopped |
| `init_advance_keeps_driving_register_only_steps_within_one_call` | sdmmc-protocol sdio/tests/io_card.rs | 单次 advance 连续推进纯寄存器步骤至首个命令（旧实现上失败已实证） |
| `drain_discard_read_failure_skips_the_round_and_keeps_draining` / `mailbox_completes_after_retrying_an_unvalidated_frame` | device/mailbox.rs tests | 排空失败容忍 + 坏帧重试后成功恢复完整链路（下一板测门禁） |

**验证**：`cargo test -p aic8800 --features "host-test rdif"`（29 单测 + 2 集成全过，原 23+2）、`cargo test -p sdmmc-protocol --features rdif`（121 全过，原 120）、`cargo xtask clippy --package aic8800/sdmmc-protocol/ax-net`（3+4+3 checks 全过）、`cargo fmt --all`。

**板测预期**：panic 点推进到 ReadRevision 邮箱——坏帧重试/排空（原 §4 定位的"邮箱阶段预防性加固"）将在那里接管；`aic: mailbox frame did not validate; retrying:` warn 与 5s `MailboxTimeout` 兜底给出可定位证据。板测前需解决串口捕获问题（executor 日志当前不可见，count raw 观察点依赖它）。
