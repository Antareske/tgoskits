# SG2002 开机自动 AP 排查记录

日期：2026-09-12 ｜ 板子：LicheeRV Nano (SG2002)，无线模块 **AIC8800D80**
源码：`/home/asta/tgoskits/tgoskits` @ `dev` `59de8ccb3`
串口日志：`/home/asta/tgoskits/logs/dev.log`（191 行，本次上板）

---

## 0. 结论速览

1. **控制面全通**：驱动把 `MM_ADD_IF → MM_START → MM_SET_FILTER → APM_SET_BEACON_IE → APM_START`
   五条命令全部发完，且每条都收到 message id 匹配、status=0 的 CFM —— **固件层面认为 AP 已经启动**。
2. **现象**：手机/笔记本扫描不到 `StarryOS-AP`，空口没有任何 beacon。
3. **两个已确认的结构性缺口**（与"没有 beacon"是否直接相关待验证）：
   - **缺口 A**：统一驱动里**完全没有 AP 客户端路径**（没有 `ME_STA_ADD`、没有主机回 AssocResp、
     没有给客户端开控制端口）。即使 beacon 出来，客户端也关联不上。旧 DC 实现有完整的 AP worker。
   - **缺口 B**：D80 的固件供给**只上传了主镜像** `fmacfw_8800d80_u02.bin`；上游同目录还有
     `fw_adid_*`（RF 校准）、`fw_patch_*`、`fw_patch_table_*`、`lmacfw_rf_*`、`aic_powerlimit_*`
     等文件，DC 路径下这些是齐的。若是必发项，则正好解释"MAC start 成功、PHY 不出信号"。
4. **下一步**：先做 STA 对照实验（判射频/数据面是否可用），再按需加诊断日志、补固件。

---

## 1. 产物与配置

| 项 | 值 |
|---|---|
| 镜像 | `/home/asta/tgoskits/wt-sg2002-wifi-opt/www/images/sg2002_starryos_wifi_ap_iperf3.img`（2,578,448,384 B） |
| sha256 | `3907ee276ad3354f06bf582ab2ae473c0f45a87a21ba807064ac6d60e181f304` |
| 内核 | dev `59de8ccb3` + `os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml` |
| rootfs | tgosimages v0.0.13 `rootfs-riscv64-alpine.img`（Alpine 3.23.5，apk 源=清华） |
| 注入 | `iperf3` 3.19.1（riscv64，来自 Alpine v3.23 main apk）+ `libiperf.so.0` → `/usr/bin`、`/usr/lib` |
| DTB | `/home/asta/tgoskits/wt-sg2002-wifi-opt/www/sg2002/wifi-ap/licheerv-nano-sg2002-ap.dtb`（源 `lcn.dts`） |
| AP 参数 | SSID `StarryOS-AP`、ch6、`192.168.50.1/24`、DHCP 单客户端 `192.168.50.2` |

DTB 开关（dev 主线**默认不开 AP**，必须显式配置；解析在
`drivers/ax-driver/src/net/aic8800/fdt.rs:145-183`），加在 `wifi-sd@4320000`
（`cvitek,cv181x-sdio`）节点：

```
aic,startup-mode   = "access-point";
aic,ap-ssid        = "StarryOS-AP";
aic,ap-channel     = <0x06>;
aic,ap-prefix-length = <0x18>;
aic,ap-ipv4        = <0xc0a83201>;   // 192.168.50.1
aic,dhcp-client-ipv4 = <0xc0a83202>; // 192.168.50.2
```

---

## 2. 日志证据链（`logs/dev.log`）

| 行号 | 时间 | 内容 | 出处 |
|---|---|---|---|
| 31 | 18.921 | `detected supported AIC SDIO variant Aic8800D80` | `rdif/owner/progress.rs:421` |
| 32 | 20.838 | `control result queued for network runtime` | `rdif/owner/output.rs:228` |
| 34 | 20.838 | `control result consumed by network runtime` | `rdif/device/endpoints/control.rs:95` |
| 45 | 20.841 | `dev 0: DHCP server enabled (lease 192.168.50.2)` | `ax-net/src/service.rs:579` |
| 184-186 | 55.9 | `wlan0: ... UP` / `inet 192.168.50.1/24` | `ip addr`（用户手动执行） |

### 证明"控制面全通"的三条推理

1. **startup 阶段全部通过**：日志中没有 `log_startup_confirmation_error`
   （`device/startup/mod.rs` 的该函数在每个阶段 CFM 的长度 / message id / status / padding 校验失败时打印），
   说明 D80 的全部启动阶段（`startup/mod.rs:41-69` 的 stage 列表：UploadMain → D80Patch →
   StartApplication → Stabilize → Reinitialize → StackStart → TxPowerLevel → RfCalibration →
   ReadMacAddress → FirmwareReset → ConfigureMac → ConfigureChannels → AddStationInterface →
   StartMac → SetFilter → ArmChipInterrupt）都跑完了。
2. **FDT 的 startup 事务确实提交了**：接口以 `mode: static` + `192.168.50.1/24` + DHCP server 发布，
   而这些值来自事务的 `link_policy`（`net/ax-net/src/lib.rs:426-441`；`initial_wifi_policies` 只在
   `queue_runtime/mod.rs:785-795` 提交事务时写入）。**注意**：静态地址来自"策略"而非"控制结果"，
   所以它本身不能证明 AP 起没起。
3. **事务以 Ok(Complete) 结束，不是失败**：任何失败都会命中
   `net/ax-net/src/queue_runtime/executor/wifi.rs:139` 的 `log::error!("Wi-Fi owner transaction failed: ...")`
   （以及设备的 `log_startup_confirmation_error`）。日志里两条都没有。
   同时 `#2276` 的 `parse_ap_start()` 还校验了 `apm_start_cfm` 的 status、vif 匹配、
   channel/BCMC 索引 —— 也就是说 **FW 回的确认内容是自洽可信的**。

> 结论：问题不在"命令没发出去/被拒"，而在"FW 说 OK 了但空口没有东西"。

---

## 3. 缺口 A：统一驱动没有 AP 客户端路径

旧实现（DC 上实测"能广播 + 能关联 + SSH 通"，见 git 历史
`2c8bd19c1:components/aic8800/src/fdrv/thread/ap.rs` 与 `.../protocol/apm.rs`）在
`APM_START` 之后还有一个 **AP worker**：

1. RX 线程收到 `AssocReq` → 入队；
2. AP worker 解析 SupportedRates → `ME_STA_ADD_REQ` 注册客户端，拿固件分配的 `sta_idx`；
3. **主机自己构造并发送 `Assoc Response`**（status=0，带 AID）；
4. `ME_SET_CONTROL_PORT_REQ(sta_idx, authorized=true)` 打开控制端口 ——
   开放网络也必须显式授权，否则固件只放行 EAPOL、丢弃该客户端所有 DHCP/ARP/IP 帧。

而在 `drivers/net/aic8800` 中：

- `ME_SET_CONTROL_PORT_REQ` **只用在 STA 自身连接流程**（`device/control.rs:131`，WPA2 握手后给
  自己的 station 开端口），不是给 AP 客户端用的；
- 全树没有 `ME_STA_ADD`，没有 AssocReq/AssocResp 处理，没有 AP 事件处理。

→ 这正是设计文档 `docs/design/unified-sdio-aic8800.md:202` 那句
「这保证 AP 启动确认可信，**不代表已经实现或验证 AP 客户端关联及完整 AP 数据面**」的实质。

**影响**：beacon 问题解决之后，关联/数据面仍需单独实现（可参照上面的旧 DC worker，
以及 vendor `change_station(AUTHORIZED)` 的语义）。

---

## 4. 缺口 B：D80 固件供给只有主镜像

`drivers/net/aic8800/build.rs:83-86` 的 manifest 里，D80 只有一项
`fmacfw_8800d80_u02.bin`。上游 `lxowalle/aic8800-sdio-firmware @ c56f910` 的
`aic8800_and_aic8800D80/` 目录实有：

```
fmacfw_8800d80_u02.bin            ← 我们唯一上传的
fmacfw_8800d80_u02_ipc.bin
fmacfw_8800d80_h_u02.bin / _ipc.bin
fmacfw_rf.bin / lmacfw_rf_8800d80_u02.bin
fw_patch_8800d80_u02.bin / _ext0.bin
fw_patch_table_8800d80_u02.bin
fw_adid_8800d80_u02.bin           ← ADID（RF 校准/功率数据）
fmacfwbt_8800d80_u02.bin          （蓝牙）
aic_userconfig_8800d80.txt / aic_powerlimit_8800d80.txt  ← 每信道 TX 功率上限表
```

对照 DC：`fmacfw_patch_*`、`fmacfw_patch_tbl_*`、`fmacfw_calib_*` 都在 manifest 里逐个列出。

D80 路径改为在 `StartApplication` 前用 debug mailbox 写三组 patch config
（`device/startup/d80.rs:22-24` 的 `PATCH_PAIRS`：2.4G only / AMSDU_RX /
power calibration + channel TX power limits），声称与 pinned Sipeed BSP 的编译选项一致。
另有 `TxPowerLevel`（`lmac.rs:224`，写 6 组 14–20 dBm profile）与
`RfCalibration`（`lmac.rs:249`，D80 走 `DualBand`）两个 LMAC 阶段。

**待确认的假设**：vendor/Sipeed 的 D80 SDIO 流程里，`fw_adid`（校准）与
`aic_powerlimit`（功率上限）是否为必发项。若是，当前状态就是"MAC 层 start 成功、
PHY 未校准/无功率表 → 空口无信号"，与观测完全吻合。

---

## 5. 下一步（按性价比）

### 5.1 STA 对照实验（镜像已构建，待上板）

用同一份驱动编**编译期 STA** 镜像（`STARRY_WIFI_SSID` / `STARRY_WIFI_PASSWORD`，
**不配** AP DTB），看能否连上目标 AP 并 DHCP：

- 能连上 → 射频/数据面 OK，问题锁定在 AP 特有路径（beacon 模板 / vif / 定时）；
- 连不上 → 问题在 D80 的固件供给/RF 标定，与 AP 无关（缺口 B 优先级提高）。

已构建（2026-09-14，dev `c1f5be737`）：

| 项 | 值 |
|---|---|
| 镜像 | `/home/asta/tgoskits/wt-sg2002-wifi-opt/www/images/sg2002_starryos_wifi_sta_iperf3.img`（在 AP 版镜像上 swap 内核 + DTB，rootfs 不变，含 iperf3） |
| 编译凭据 | `STARRY_WIFI_SSID='luori'` / `STARRY_WIFI_PASSWORD='12345678'`（`option_env!` 编入，`starryos.bin` 中可见 `luori`） |
| DTB | `/home/asta/tgoskits/wt-sg2002-wifi-opt/www/sg2002/wifi-sta/licheerv-nano-sg2002-sta.dtb`（stock DTB + `/chosen/rng-seed`） |
| 种子 | `cb874a09f1722837e471ba489d1b696100e103435fe73e62ee9bdfabb68ef8d3`（32 字节，镜像级一次性；换镜像应换种子） |

两条关键约束（都来自代码，非猜测）：

1. **种子必须落在镜像的 DTB 里**。板测流程由 `scripts/axbuild/src/starry/boot_entropy.rs`
   往临时 DTB 副本注入 32 字节 `/chosen/rng-seed`；静态烧写的镜像没有这个环节，
   而 WPA2 连接缺少可信种子会 fail closed（`EntropyUnavailable`）。
   `someboot` 只接受**精确 32 字节**（`platforms/someboot/src/fdt/mod.rs:65`）。
2. **STA DTB 不能带 `aic,startup-mode`**。编译期 station 策略与 FDT 启动策略同时存在时
   probe 显式拒绝（`drivers/ax-driver/src/net/aic8800/fdt.rs:126-140`），
   所以 STA 版用 stock DTB（已确认 `aic,` 属性计数为 0）。

预期日志：`[wifi] secure startup connection entropy prepared` → 关联 →
`DHCP acquired address <x>`；失败看 `Wi-Fi owner transaction failed: ...`。

### 5.2 加三行诊断日志再上板（4 min 编译 + 47 s 组装）

建议在驱动里补：

1. `MM_ADD_IF_CFM` 解析出的 **vif index**（验证 #2276 的实际输入是 1 还是 0）；
2. `APM_START_CFM` 的 4 字节 payload（status / vif / channel / BCMC）；
3. `APM_SET_BEACON_IE_CFM` 对应的 bcn_len / tim_oft（确认 FW 收到的 beacon 模板长度）。

位置：`device/control.rs` 的 `confirm_ap_start()` / `assign_ap_interface()`，
以及 `device/mailbox.rs` 的 CFM 分发处。

### 5.3 补齐 D80 固件供给

需要 vendor/Sipeed 的 D80 SDIO 启动顺序（哪些 blob、上传地址、顺序、是否走 debug mailbox）。
拿到后改 `build.rs` manifest + `device/startup/firmware.rs` 的 D80 分支。
（网络：本机 GitHub 只有 SSH / raw 可用。）

---

## 6. 复现命令

```bash
# 1) 内核（wifi 配置）
cd /home/asta/tgoskits/tgoskits
cargo xtask starry build -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml

# STA 版在此基础上加编译期凭据（不带 aic,startup-mode 的 stock DTB）：
# STARRY_WIFI_SSID='luori' STARRY_WIFI_PASSWORD='12345678' \
#   cargo xtask starry build -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml

# 离线/网络抖动时，aic8800 的 build.rs 会因拉取固件失败而中断
# （raw.githubusercontent 连接被重置）。指向本机已校验的缓存即可，
# build.rs 仍会逐个按 pinned sha256 校验：
# AIC8800_FIRMWARE_DIR=<上一次构建的 OUT_DIR>/firmware cargo xtask starry build ...

# 2) 组装（AP DTB + iperf3 注入）
/home/asta/.claude/skills/sg2002-image-build/scripts/sg2002-image-build.sh build \
  --source /home/asta/tgoskits/tgoskits \
  --config os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml \
  --dtb /home/asta/tgoskits/wt-sg2002-wifi-opt/www/sg2002/wifi-ap/licheerv-nano-sg2002-ap.dtb \
  --kernel /home/asta/tgoskits/tgoskits/target/riscv64gc-unknown-none-elf/release/starryos.bin \
  --rootfs /tmp/sg2002-rootfs/rootfs-riscv64-alpine.img \
  --inject /tmp/iperf3-inject/iperf3:/usr/bin/iperf3:0755 \
  --inject /tmp/iperf3-inject/libiperf.so.0:/usr/lib/libiperf.so.0:0755 \
  -o /home/asta/tgoskits/wt-sg2002-wifi-opt/www/images/sg2002_starryos_wifi_ap_iperf3.img

# 3) 校验（工作区版工具，32 项）
/home/asta/tgoskits/sg2002-image-build/scripts/verify-image.sh <镜像> \
  --assets /home/asta/tgoskits/sg2002-image-build/assets \
  --expect-kernel .../starryos.bin --expect-dtb .../licheerv-nano-sg2002-ap.dtb \
  --expect-rootfs .../.sg2002-build/assets/rootfs.ext4 \
  --expect-fip /home/asta/tgoskits/sg2002-image-build/assets/fip.bin \
  --expect-uimg /starryos.uimg \
  --expect-rootfs-file /usr/bin/iperf3 --expect-rootfs-file /usr/lib/libiperf.so.0

# 4) 烧写
sudo dd if=<镜像>.img of=/dev/mmcblk0 bs=4M status=progress conv=fsync
```

**不要**用构建流程里的 `--init-assets`：它的 `starry-init.sh` 会对 wlan0 跑 `udhcpc`，
在 AP 模式下语义相反（串口 shell 由内核 PID 1 的 `/bin/sh -l -i` 直接提供，不依赖 getty）。

---

## 7. 关键代码位置索引

| 主题 | 位置 |
|---|---|
| FDT 启动策略解析（`aic,startup-mode` 等） | `drivers/ax-driver/src/net/aic8800/fdt.rs:145-183` |
| 启动事务提交 + 策略记录 | `net/ax-net/src/queue_runtime/mod.rs:785-795` |
| 策略 → 静态地址 / DHCP server | `net/ax-net/src/lib.rs:426-441`、`service.rs:579` |
| 控制事务失败可见性（error 日志） | `net/ax-net/src/queue_runtime/executor/wifi.rs:139` |
| AP 命令序列（MM_ADD_IF/MM_START/filter/SET_BEACON_IE/APM_START） | `drivers/net/aic8800/src/device/control.rs:407+` |
| D80 启动阶段机 | `drivers/net/aic8800/src/device/startup/mod.rs:41-69`、`startup/firmware.rs:75-89` |
| D80 patch config（三组） | `drivers/net/aic8800/src/device/startup/d80.rs:22-24` |
| D80 AP 控制序列单测（用 Aic8800D80 跑） | `drivers/net/aic8800/src/device/mailbox.rs:699+` |
| 旧 DC 版 AP 实现（可参考） | `git show 2c8bd19c1:components/aic8800/src/fdrv/thread/ap.rs`、`.../protocol/apm.rs` |
| 维护者关于 AP 范围的表述 | `docs/design/unified-sdio-aic8800.md:202` |
