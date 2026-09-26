## 背景

SG2002 (LicheeRV Nano) 板载 Aic8800D80 WiFi（SDIO1）。主线 SDIO 协议统一重构（#2201）将 sdmmc-protocol 与 aic8800 驱动重组为"被动状态机核心 + rdif 能力适配层"的新结构。重构后的驱动在板上初始化失败，问题按板测顺序逐层暴露：初始化超时 → 启动失败原因不可见 → 写寄存器响应形状失配 → 芯片版本解析错误 → 设备 MAC 缺失。本 PR 修复这条启动链上的全部缺陷，使 SG2002 WiFi 驱动恢复到开机可用状态。

## 平台

SG2002 荔枝派，aic8800d80

## 要解决的问题与修复

### 1. SDIO 初始化纯寄存器步骤死锁（sdmmc-protocol）

`advance_init_request` 提交下一个步骤后返回 Pending，协议层按"等待完成中断"处理。但纯寄存器总线操作（CMD52、总线宽度/时钟设置）从不产生中断，初始化在首个命令步骤之前死等到 60s 启动超时。板测表现为 67s panic。

修复：每次新提交的步骤立即以 `ProgressCause::Submitted` 推进一次，直到进入首个命令步骤（CMD5）才等待完成中断；只有单次调用的首次推进使用调用方提供的中断原因。

回归测试 `init_advance_keeps_driving_register_only_steps_within_one_call` 断言单次推进后 CMD5 已提交、后续步骤均以 Submitted 驱动（MockHost 记录每次总线推进的原因）。

### 2. 启动失败原因不透传（ax-net）

网络队列启动失败时 `QueueInit` 不携带错误原因，panic 文本为泛化的 "network queue initialization failed"，板上无法定位根因。

修复：`QueueInit(NetError)` 携带真实失败原因——executor 在初始化失败时记录 `startup_error`，runtime builder 取回后随 panic 报告。后续每一轮板测定位（响应形状失配、版本解析错误）都依赖这条错误链。

### 3. 写寄存器响应形状契约（aic8800）

rdif 适配层将每个 Direct 操作（含写）的完成统一映射为 `Byte`——CMD52 的 R5 响应恒携带数据字节，且驱动以 `read_after_write` 提交。核心侧四个写寄存器消费点仍以 `expect_unit` 期待 `Unit`，确定性触发 `MalformedResponse` panic。失配点共 4 处：`VendorSetup`、`Reinitialize`、`ArmChipInterrupt`（启动序列）与 `Shutdown`（停机序列）。

修复：新增 `expect_write_ack`（接受 `Byte`、丢弃读回值），4 个消费点统一改用；`request.rs` 固化"请求类型 → 完成形状"契约表；`MalformedResponse` 的显示文案改为形状中立措辞（原文案提及 mailbox，会把排查引向错误方向）。

4 条阶段注入回归测试覆盖全部失配点（接受侧断言推进、拒绝侧喂 `Unit` 断言失败）。

### 4. ReadRevision 版本号取低半（aic8800）

ReadRevision 读回字的版本号位于高半（`CHIP_REV_HIGH_SHIFT` 位移之后），低半是无关值噪声。新代码直接取低 6 位，板上得到 0x20（32）并 panic "unsupported chip revision"。

修复：按 `(raw >> CHIP_REV_HIGH_SHIFT) & CHIP_REV_MASK` 解析，校验改用 `CHIP_REV_U01/U02/U03` 常量。回归测试以板上实际噪声值（0x0001_0020）钉住解析行为。

### 5. 启动期从固件读取设备 MAC（aic8800）

设备 MAC 从未被获取，`Started` 事件与后续 LMAC 命令携带全零地址（板上 `eth0` 显示 00-00-00-00-00-00）。

修复：启动序列新增 `GetMacAddress` 阶段（StackStart 之后、中断部署之前），发送 `MM_GET_MAC_ADDR_REQ`（0x0073）并从确认帧载荷前 6 字节安装 MAC；载荷不足 6 字节时按 MalformedResponse 失败。板测确认 MAC 正确读回并随 `Started` 事件发布。

### 6. 设备失败事件可靠传播（aic8800）

设备进入 Failed 后，失败事件经 progress 队列发布，队列背压时事件丢失，控制请求调用方要等 30s 兜底超时（`DEFAULT_CONTROL_TIMEOUT`）才收到错误。

修复：失败事件直接向上返回错误，不再经过 progress 队列。板测确认失败在邮箱超时后立即返回。

## 验证

- 单元与集成测试：aic8800 28 单测 + 2 集成、sdmmc-protocol 121，全部通过
- 静态检查：`cargo xtask clippy`（aic8800 / sdmmc-protocol / ax-net）、`cargo fmt --all --check` 干净
- 板测（LicheeRV Nano）：WiFi feature SG2002 开机推进到 CLI，启动链完整走通；`ip addr` 显示设备真实 MAC（38-7a-cc-9b-2c-c8）

## 范围说明

本 PR 只包含初始化启动链修复，目前 aic8800d80 可以开机不 panic。板测过程中定位到但超出本 PR 范围的问题（运行期 AP/STA 切换时固件对首条 LMAC 命令无应答）未包含在内：串口登录后仍无法开启 WiFi ap/sta，ioctl 失败且立即出现串口逐字符显示的 CPU 卡顿现象，因此未测试 WiFi 工作是否正常；不能确定该现象是否是本 pr 改动导致的新问题，因为主线 starry 在开机的 WiFi 初始化阶段即 panic，没有成功先例作为对照。