# dev 主线 931.log panic 分析：V3 wakeup 写入回读值校验失配

- 日期：2026-09-03
- 板测材料：`www/archive/9-4-after-wifi-refactor/logs/931.log`（2026-09-03 15:44 拍摄，log副本在www/logs/931.log，以后日志统一放www/logs）
- 对照材料：`www/archive/9-4-after-wifi-refactor/logs/dev-wifi.log`（dev 早期板测，同一失败点）
- 相关：`www/archive/9-4-after-wifi-refactor/pr-rsp.md`（PR 关闭反馈）、`www/newdev/VendorSetup响应形状失配.md`（前一形态的失配记录）

---

## 0. 摘要

dev 主线 931.log 的 panic 真因是 **#2222 新增的 CMD52 RAW 写回读值校验**：V3 启动序列
第三步（wakeup 寄存器 0x02 写 0x11）完成时读回字节为 0x01，而 dev 的
`validate_vendor_setup_readback` 要求读回值等于写入值 0x11，判定失配 →
`SdioWriteReadbackMismatch { expected: 0x11, actual: 0x01 }` → 启动失败 →
`axruntime/src/devices.rs:97` panic。

本分支/initFix PR 的"写寄存器形状契约"（`expect_write_ack`，只校验 Byte 形状、
丢弃读回值）恰好是这一点的修复；#2222 在形状校验之上"进一步校验读回值"
（pr-rsp 第 10 行），其 D80/V3 路径从未在实板上验证过——AKA 实板是 DC 芯片
（CIS c8a1:c08d），走 V1 序列，不包含 wakeup 写。因此 dev 在用户板卡
（LicheeRV Nano，Aic8800D80）上确定性 panic。

---

## 1. 证据链

### 1.1 板上内核确为 dev 主线构建

- 931.log FIT kernel `crc32 = dd328f38`、Data Size 14954400，与
  `/workspace/tgoskits/.sg2002-build/assets/starryos.bin`（2026-09-03 08:34 构建，
  源码为 dev HEAD 15dee37d2，工作区干净）逐字节一致（crc32 计算比对通过）。
- panic 文本带 `QueueInit(NetError)` 完整错误链（`failed to initialize network
  queue runtime: network queue initialization failed: Other error: AIC core
  failed: AIC CMD52 RAW readback mismatch: expected 0x11, received 0x01`），
  为 #2222 合并后的新错误格式。

### 1.2 失败点定位（代码事实）

dev HEAD 上所有带值校验的寄存器写（`expect_write_readback` 调用点全量枚举）：

| 位置 | 期望值 |
|---|---|
| `device/startup/vendor.rs:59` V3 index 0 | 0x7f（fn0 0xf2） |
| `device/startup/vendor.rs:59` V3 index 1 | 1（fn1 byte_mode_enable 0x07） |
| `device/startup/vendor.rs:59` V3 index 2 | **0x11（fn1 wakeup 0x02，`SDIOWIFI_V3_WAKEUP_VALUE`）** |
| `device/startup/vendor.rs:59` V3 index 3（reinitialize） | 0x07（fn0 0x04） |
| `device/startup/vendor.rs:59` V1 index 0-5 | 1/1/1/1/0x07/0x07 |
| `device/startup/mod.rs:423` ArmChipInterrupt | `INTERRUPTS_ENABLED` = 0x07 |
| `device/progress.rs:171` Shutdown | 0 |

dev 上全量 `write_byte(` 调用点中，唯一写入值 0x11 的是 V3 wakeup 写
（`common/mod.rs:70` `SDIOWIFI_V3_WAKEUP_VALUE: u8 = 0x11`）。panic 的
`expected: 0x11` 只能来自该步 → 失败点为 **V3 VendorSetup(2)**，且前两步
（0x7f、1）的值校验在板上通过（板卡确实走到第三步才失败）。

### 1.3 板上跑的是 V3/D80 序列

- 交接文档（2026-08-29）明确记录板卡为"LicheeRV Nano（SG2002），板载
  Aic8800D80 WiFi（SDIO1）"。
- 旧 dev 的 `chip_variant()`（`ax-driver/src/net/aic8800/fdt.rs`）在
  `aic,chip-variant` 缺失时默认 D80，且旧 dev 的 `validate_card_identity`
  要求 CIS 检测与设备变体一致——旧 dev 板测能过身份关卡并走到 VendorSetup，
  证明板卡 CIS 报 D80（c8a1:0082）。
- 新 dev 改为 CIS 检测（`rdif/owner/progress.rs: detect_sdio_card_variant`），
  板上仍走 V3 序列（panic 本身即证据）。

### 1.4 wakeup 写入"生效"与"读回值非写入值"的证据

- 厂商驱动对照（`www/newdev/问题修复追踪.md` 记录，aicsdio.c:749-790
  `aicwf_sdio_wakeup`）：D80 写 wakeup_reg(0x02)=0x11，然后**轮询
  sleep_reg(0x01) bit4(0x10)** 判断固件 ready；厂商驱动从不校验 wakeup 写的
  读回值。
- dev 自身已实现厂商式就绪校验：`VendorSetup(3)→None` 后进入
  `VendorDelay`（`FUNCTION_READY_DELAY_V3`）→ `VendorReady` 读 sleep_status
  并检查 `SLEEP_STATUS::READY`（bit4=0x10）。wakeup 写是否生效的权威判定点
  是 VendorReady，不是该写的 RAW 读回值。
- 本分支（initFix 修复后）板上实测：同一 wakeup 写以形状校验通过后，
  VendorDelay→VendorReady 的 sleep-status bit4 检查通过、ReadRevision 邮箱
  GET_MAC CFM 到达、启动推进至 Ready（wc3.log 及交接文档记录）——证明
  wakeup 写在该硬件上确实生效，读回字节 0x01 不代表写失败。
- SDIO 规范语义：CMD52 RAW=1 的响应数据域为**写前**寄存器值；wakeup 写前
  芯片处于浅睡状态、寄存器为 0x01 时，读回 0x01 是规范允许的合法结果。
  要求其等于写入值 0x11 在该寄存器上无规范依据。

### 1.5 为什么 AKA 验证没暴露

设计文档 `docs/design/unified-sdio-aic8800.md` 记录："AKA 实板的 common CIS
为 c8a1:c08d，对应 AIC8800DC，而不是原先假定的 D80 c8a1:0082"。#2222 的
exact-head AKA 板级验证（WPA2/DHCP/iperf）在 DC 上运行 → V1 profile →
V1 vendor 序列（1/1/1/1/0x07/0x07），**V3 wakeup 值校验从未在实板上执行过**。
用户板卡为 D80（V3 序列），于是第一个板测即确定性失败。

---

## 2. 与 PR/initFix 修复的对应关系

| PR 修复 | dev 现状 | 与本次 panic 关系 |
|---|---|---|
| 写寄存器形状契约（`expect_write_ack`，4 点） | #2222 改为形状+值校验（`expect_write_readback`） | **直接相关**：值校验正是 panic 点；形状部分 dev 已有 |
| init 循环推进（92a5a329） | pr-rsp 列为未吸收，但 dev 的 #2222 实现自带驱动循环 | 无关（本 panic 在响应到达后发生） |
| fail 事件绕过 progress ring（ed354c7c） | pr-rsp 列为未吸收 | 无关（本次是启动期确定性失败，非背压延迟） |

pr-rsp 的"尚未吸收的范围"仅列了后两项；本次 panic 揭示第三项：
**写回读值校验在 D80/V3 wakeup 寄存器上不成立**——pr-rsp 第 10 行
"#2222 进一步校验读回值"的加强版正是回归来源。

---

## 3. 修复方向

范围单一原则（pr-rsp 指引：基于最新 dev 接口提取，保留确定性回归测试）：

1. **最小修复（推荐）**：`validate_vendor_setup_readback` 对 V3 wakeup 写
   （index 2、非 reinitialize）退化为形状校验（接受 Byte、丢弃值），
   注释说明：该寄存器的 RAW 读回值为写前值/芯片状态，厂商驱动不校验，
   就绪判定由 VendorReady 的 sleep-status bit4 负责。其余写点的值校验保留。
2. **契约级修复（备选）**：恢复 PR 的"写 ack 形状契约"——所有启动期写
   ack 一律只验形状。范围更大，与 #2222 的加强方向相反，需要更强的
   论据（可作为后续板测暴露更多失配点后的兜底）。
3. **回归测试（测试先行）**：阶段注入测试——V3 profile 设备置于
   `VendorSetup(2)`，喂 `SdioResponse::Byte(0x01)`，断言推进
   `VendorSetup(3)` 而非 `Failed`。当前 dev 实现上该测试必然失败（与
   panic 同构）。

### 后续风险点（本报告未实测，板测注意）

- **Reinitialize(3)**（写 fn0 0x04=0x07，值为 `INTERRUPTS_ENABLED`）：
  fn0 0x04 是 `SDIOWIFI_MISC_INT_STATUS_REG_V3`（W1C 状态寄存器），
  写后读回大概率非 0x07——dev 的值校验可能在此再次 panic。
- **ArmChipInterrupt**（写 fn1 0x00=0x07）：固件运行后中断使能寄存器的
  读回行为未在本板上验证。
- 建议最小修复落地后先做一次板测：若上述两点出现同类失配，再按证据
  逐点放宽（每次一条，带各自回归测试）。

---

## 附录：关键位置索引（dev HEAD 15dee37d2）

- `drivers/net/aic8800/src/device/startup/vendor.rs:13-59` — V3/V1 vendor 写序列与值校验
- `drivers/net/aic8800/src/device/request.rs:23-31` — `expect_write_readback`（失配判定）
- `drivers/net/aic8800/src/device/model.rs:330` — `SdioWriteReadbackMismatch` 错误
- `drivers/net/aic8800/src/device/startup/mod.rs:160-200` — VendorDelay/VendorReady（就绪权威判定）
- `drivers/net/aic8800/src/common/mod.rs:70` — `SDIOWIFI_V3_WAKEUP_VALUE = 0x11`
- `drivers/net/aic8800/src/rdif/owner/progress.rs:514-528` — CIS 变体检测
- `docs/design/unified-sdio-aic8800.md` — AKA 实板 CIS = c8a1:c08d（DC）
- `os/arceos/modules/axruntime/src/devices.rs:97` — panic 点
