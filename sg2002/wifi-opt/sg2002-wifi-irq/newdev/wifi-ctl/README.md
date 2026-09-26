# wifi-ctl：StarryOS (SG2002 aic8800) WiFi 模式控制工具

在板端（StarryOS 用户态）切换无线接口的 Station（STA）/ SoftAP（AP）模式。

## 背景

- 驱动启动默认模式由设备树 `aic,startup-mode` 属性决定（缺省为"空闲"，即既不 STA 也不 AP）；**启动即 STA 不受支持**，STA 只能运行时切换。
- StarryOS 在 socket fd 上实现了 Linux wireless-extensions ioctl 子集（`os/StarryOS/kernel/src/file/wext.rs`），stage-then-commit 语义：`SIOCSIWMODE`/`SIOCSIWESSID`/`SIOCSIWENCODEEXT`/`SIOCSIWFREQ` 只暂存，`SIOCSIWCOMMIT` 原子提交（链路拆除 + 切换 + IP/DHCP 角色一并完成）。
- AP 模式的运行时策略由内核固定：IP `192.168.50.1/24`、DHCP client `192.168.50.2`、默认 channel 6（wext.rs 常量，与 boot 期 SoftAP 策略一致）。

## 用法

```
wifi-ctl <ifname> sta <ssid>            # 开放网络（无密码）
wifi-ctl <ifname> ap  <ssid> [channel]  # 开放 AP，channel 默认 6
```

示例：

```
# 查看接口名（接口名 = 网络栈侧命名，当前镜像下为 eth0，不是驱动注册名 wlan0）
ip addr

# 切换为开放 AP
wifi-ctl eth0 ap SG2002 6

# 切回开放网络 STA
wifi-ctl eth0 sta OpenNet
```

## 已知限制

- **WPA2 STA 暂不可用**：带密码的 STA 连接会被内核无线提交路径拒绝（`WifiTransaction::connect` 未携带 WPA 熵，驱动返回 `EntropyUnavailable`）。工具对带密码的 `sta` 前置拒绝并提示；接入内核熵源后方可恢复。
- **不支持"断开"**：内核 wext 子集未暴露 disconnect；断开可用切换到另一模式替代。
- **并发**：stage 暂存表是内核全局的——两个 wifi-ctl 进程并发时 stage/commit 可能交错产生混合配置。单用户场景无碍；脚本化并发调用请串行执行。
- **中途失败**：stage 途中 ioctl 失败会留下半暂存配置；下一次成功运行会逐字段覆盖（自愈），但提交失败后接口实际状态不可见（无查询 ioctl）。

## 编译（宿主机）

```sh
./build.sh
# 产物：wifi-ctl（riscv64 musl 静态链接）
```

工具链：`/opt/riscv64-linux-musl-cross/bin/riscv64-linux-musl-gcc`（可用 `CC=...` 覆盖）。构建期 `_Static_assert` 钉住 `struct iwreq` 的 ABI 布局（32 字节、iw_point 指针 8 字节）。

## 注入 rootfs（随镜像构建）

用 sg2002-image-build skill 的注入配方：

```
--inject www/newdev/wifi-ctl/wifi-ctl:/usr/bin/wifi-ctl:0755
```

## 协议细节（与内核侧对照）

| ioctl | 值 | 载荷 | 内核行为 |
|---|---|---|---|
| `SIOCSIWMODE` | 0x8B06 | iwreq_data 前 4 字节 = 2(STA)/3(AP) | 暂存模式 |
| `SIOCSIWESSID` | 0x8B1A | iw_point { ptr, len≤32, flags } | 暂存 SSID |
| `SIOCSIWENCODEEXT` | 0x8B34 | iw_point { ptr, len≤63 }（取 passphrase 原文） | 暂存 PSK |
| `SIOCSIWFREQ` | 0x8B04 | iwreq_data 前 4 字节 = channel(1-14) | 暂存 channel（仅 AP） |
| `SIOCSIWCOMMIT` | 0x8B00 | 仅接口名 | 原子提交 → `ax_net::reconfigure_wifi` |

`struct iwreq` 固定 32 字节（16 字节 ifrn_name + 16 字节 union），32/64 位一致。
