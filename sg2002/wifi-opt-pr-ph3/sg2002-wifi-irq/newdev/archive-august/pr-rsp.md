## 关闭原因

#2222 已于 2026-09-01 合并到 `dev`（merge commit `9093023e2ebe5e02867e8538e112687c23507f35`），并在新的统一 SDIO/AIC8800 所有权与运行时结构上完成了 AKA Wi-Fi 启动、异步关联、WPA2、DHCP 和 iperf 实板验证。

本 PR 当前 head `ed354c7c1afc53aa1c1c89e56e88f2aa85582a86` 修改的 13 个文件全部也由 #2222 修改；在 #2222 合并后，本分支已与最新 `dev` 冲突。继续整单合入会重复或回退已经落地的 AIC 启动、邮箱、SDIO owner 和 `ax-net` 运行时实现，因此关闭本 PR，避免同时维护两条互相覆盖的实现路径。

## 已由 #2222 覆盖的范围

- `QueueInit(NetError)` 与具体启动错误传播；
- CMD52 Direct 写操作的 Byte readback 形状校验，且 #2222 进一步校验读回值；
- 从 readback 高半部解析 chip revision；
- 启动阶段从 firmware 读取、校验并安装 MAC 地址。

#2222 的 exact-head AKA 板级任务还完成了 WPA2、DHCP、TX/RX/双向 iperf smoke，覆盖范围已经超过本 PR 的“启动到 CLI、尚不能完成 AP/STA 切换”状态。

## 尚未吸收的范围

关闭不表示本 PR 的全部修改已经合入，也不表示以下两项被否定。当前 `dev` 仍未包含：

- `92a5a32956f3112eca09d9f3de1b65693a3fa7c1`：SDIO 初始化中纯寄存器步骤在同一次调用内以 `ProgressCause::Submitted` 连续推进，以及对应确定性回归；
- `ed354c7c1afc53aa1c1c89e56e88f2aa85582a86`：设备失败事件绕过可能背压的 progress ring，立即返回控制调用方。

如果这两项在最新 `dev` 上仍可复现，应分别基于最新接口提取为范围单一的修复，保留各自的确定性回归测试，不再复活本 PR 中已经被 #2222 取代的启动与邮箱实现。
