# SG2002 WiFi 初始化有效修复总结（PR 准备）

- 分支：`sg2002/wifi-irq` → PR 提交分支 `sg2002/wifi-irq-initFix`
- 建立：2026-08-30
- 用途：界定 PR 范围——只收录板测验证生效的功能性修复，排除板测未见效的修复（后者保留在 sg2002/wifi-irq 分支继续调试）
- 板测日志依据：www/logs/ 1.log-8.log、wc2.log-wc6.log
- 备注：全部新增诊断日志已从 PR 移除（dev 无任何对应日志，"降级防刷屏"是相对分支自身先前状态而非 dev，不构成 PR 修复项）

---

## PR 范围（6 项功能性修复）

### 1. sdmmc-protocol：SDIO 初始化纯寄存器步骤同步推进 —【板测生效】

- **问题**：`advance_init_request` 提交纯寄存器总线操作（CMD52/总线设置）后返回 Pending，协议层按"等中断"处理，而寄存器操作从不产生中断 → 死等到 60s 超时 panic（1.log 67s 实证）。
- **修复**：提交新步骤后立即以 `ProgressCause::Submitted` 推进，直到第一个命令步骤（CMD5）才等待完成中断。
- **文件**：`drivers/blk/sdmmc-protocol/src/sdio/io/init.rs`、`sdio/tests/io_card.rs`、`sdio/tests/mod.rs`（MockHost cause 记录）
- **板测证据**：5/6/7/8.log 均在 ~37ms 到达 AIC 启动（无修复必 60s stall）。
- **测试**：`init_advance_keeps_driving_register_only_steps_within_one_call`。

### 2. ax-net：启动失败原因透传 —【板测生效】

- **问题**：`QueueInit` 无 payload，panic 文本泛化为 "network queue initialization failed"，板测无法定位根因。
- **修复**：`QueueInit(NetError)` 携带真实失败原因（executor 记 `startup_error`、builder 取回后 panic）。
- **文件**：`net/ax-net/src/queue_runtime/mod.rs`、`queue_runtime/executor/mod.rs`
- **板测证据**：4/5/6/7.log 的 panic 文本均带完整错误链——历次根因定位（形状失配、revision 解析）都依赖该机制。

### 3. aic8800：写寄存器响应形状契约（4 点失配） —【板测生效】

- **问题**：核心 `expect_unit` 期待 Unit，而 rdif 适配层 Direct 完成恒映射 `Byte`（CMD52 R5 恒携带数据字节、`read_after_write: true`）→ 确定性 `MalformedResponse` panic（4/5/6.log 37-40ms 实证）。失配点 4 处：VendorSetup、Reinitialize、ArmChipInterrupt、Shutdown。
- **修复**：新增 `expect_write_ack`（接受 Byte、丢弃读回值），4 个消费点统一改用；`request.rs` 固化"请求 kind → 完成形状"契约表；`MalformedResponse` Display 改为形状中立措辞。
- **文件**：`drivers/net/aic8800/src/device/request.rs`、`device/startup/mod.rs`、`device/progress.rs`、`device/model.rs`
- **板测证据**：7.log 越过全部 4 个失配点到达 ReadRevision。
- **测试**：4 条阶段注入回归测试（含拒绝侧喂 Unit → Failed）。

### 4. aic8800：ReadRevision 版本号取高半 —【板测生效】

- **问题**：新代码 `raw & 0x3F` 取低 6 位噪声（0x20=32）→ panic "unsupported chip revision 32"（7.log 实证）。
- **修复**：`(raw >> CHIP_REV_HIGH_SHIFT) & CHIP_REV_MASK`；校验改用 `CHIP_REV_U01/U02/U03` 常量。
- **文件**：`drivers/net/aic8800/src/device/startup/firmware.rs`
- **板测证据**：8.log 接受 revision U01，开机成功。
- **测试**：`read_revision_takes_the_version_from_the_high_half_of_the_read_back`。

### 5. aic8800：启动期从固件读取 MAC 地址 —【板测生效】

- **问题**：设备 MAC 从未获取，`data.mac_address` 恒全零 → `Started` 事件携带全零 MAC（eth0 显示 00:00:00:00:00:00），且后续 ADD_IF 等命令携带全零地址。
- **修复**：StartupStage 新增 `GetMacAddress`（StackStart 之后）：发 `MM_GET_MAC_ADDR_REQ`(0x0073) 邮箱，CFM payload 前 6 字节存入 `data.mac_address`（len<6 → MalformedResponse；全零 → warn 放行）。
- **文件**：`drivers/net/aic8800/src/device/startup/mod.rs`、`device/startup/firmware.rs`、`protocol.rs`（`MM_GET_MAC_ADDR_REQ` 常量）
- **板测证据**：wc3/wc4/wc5/wc6 的 `eth0 mac: 38-7a-cc-9b-2c-c8`（GET_MAC CFM 到达 + Started 事件携带真 MAC）。
- **测试**：`get_mac_address_confirmation_installs_the_mac_and_arms_the_chip_interrupt`、`get_mac_address_confirmation_shorter_than_the_address_fails_the_device`。

### 6. aic8800：设备失败事件可靠传播 —【板测生效】

- **问题**：设备 `fail()` 后失败事件经 progress 队列 publish，背压时丢失 → 控制请求等待 30s 兜底超时（`DEFAULT_CONTROL_TIMEOUT`）才返回（wc5 70.9s→101.0s 实证）。
- **修复**：`AicEvent::Failed` 直接向上返回错误，不经 progress 队列。
- **文件**：`drivers/net/aic8800/src/rdif/owner/output.rs`
- **板测证据**：wc6 失败在 5s 内返回（26.07-21.06）。

---

## 排除出 PR（未见效，保留在 sg2002/wifi-irq 继续调试）

| 改动 | 排除理由 |
|---|---|
| AP 序列 LMAC 前置 4 命令（MM_RESET/RF_CALIB/ME_CONFIG/ME_CHAN_CONFIG，control.rs） | wc4 证明无效：第一条 MM_RESET 即无 CFM |
| Ready 态邮箱 Wake 相位（mailbox.rs） | wc5 证明无效：bit4 置位但命令不处理 |
| card interrupt int pending 清除（model.rs/data_plane.rs/operation.rs/progress.rs） | wc6 证明无效：问题不变 |
| 邮箱排空/坏帧重试加固（mailbox.rs） | 防御性改动，无对应板测问题 |
| count raw 非零 info（mailbox.rs） | 诊断临时改动，板测完成 |
| 全部新增诊断日志（startup stage/submit sdio/irq snapshot/init state/MAC warn/startup error） | dev 无任何对应日志，相对 dev 是净新增而非修复；已从 PR 移除 |
| wifi-ctl 工具（www/newdev/wifi-ctl/） | 个人目录，不入 PR |
| Cargo.lock（axhvc/axivc 移除） | 与本次修复无关 |

---

## PR 分支状态（已就绪）

- 副本分支 `sg2002/wifi-irq-initFix`（worktree `/workspace/wt-sg2002-wifi-irq-initFix`），4 条提交：
  1. `92a5a3295 fix(sdmmc-protocol): drive register-only init steps synchronously`
  2. `409f7c6e6 fix(ax-net): carry the startup cause in queue init failures`
  3. `9bfcd0f8f fix(aic8800): restore board startup after the SDIO protocol rework`（写回执契约 + revision 解析 + GET_MAC）
  4. `ed354c7c1 fix(aic8800): propagate device failure to the control caller`
- 最终 PR diff：13 文件 +442/-53（Cargo.lock 已排除；零新增日志、零中文注释、无本地路径引用）
- 验证：aic8800 28 单测 + 2 集成、sdmmc-protocol 121、clippy ×3、fmt 全过
- 待办：PR 描述撰写（中文正文）后提交 PR（推送需用户批准）
