# SG2002 Linux 侧 WiFi 驱动固件调查

调查日期：2026-09-04
调查对象：LicheeRV Nano（SG2002）板载 aic8800 WiFi，运行 vendor Linux 镜像
调查目标：定位板端 WiFi 固件资产，确认 Linux 侧的驱动加载链与实际固件组合，为 starry 侧 aic8800 驱动的固件/patch 配置对比提供参照。

## 1. 板子与软件环境

| 项目 | 值 |
|---|---|
| hostname | licheervnano-86bb |
| 系统 | Buildroot 2023.11.2（gd4003f15b） |
| 内核 | Linux 5.10.4-tag riscv64，单核 C906 |
| WiFi 设备 | `aicwf_sdio mmc1:390b:1`（SDIO，vendor/product 390b:1） |
| wlan0 | AP 模式，SSID `sg2002`，ch1（2412 MHz），20 MHz，txpower 18 dBm |

## 2. 驱动模块与加载链

加载由 `/etc/init.d/S25wifimod` 完成，按序 `insmod`，无任何模块参数：

```
cfg80211.ko → 3rd/aic8800_bsp.ko → 3rd/aic8800_fdrv.ko → 3rd/8733bs.ko
```

模块实际位于 `/mnt/system/ko/`（`3rd/` 子目录存放第三方模块）。加载后依赖关系：`cfg80211` 被 `8733bs` 与 `aic8800_fdrv` 共同使用；`aic8800_fdrv` 依赖 `aic8800_bsp`。

`aic8800_fdrv` 模块参数（sysfs 读取，加载时未显式指定，均为驱动默认值）：

| 参数 | 值 | 说明 |
|---|---|---|
| aicwf_dbg_level | 1039 | 调试日志级别位图，未开启固件路径打印位 |
| ht_on / vht_on / he_on | Y / Y / Y | 11n/11ac/11ax 使能 |
| nss | 2 | 空间流数 |
| use_2040 / use_80 | Y / Y | 40/80 MHz 带宽使能 |
| ps_on | Y | 省电使能 |

## 3. 固件资产布局

板端固件根目录为 `/usr/lib/firmware/`（`/lib` 是指向 `usr/lib` 的符号链接，两条路径等价）：

```
/usr/lib/firmware/
├── aic8800_sdio/
│   ├── aic8800/                    # 8800 原版固件（u03 世代）
│   ├── aic8800D80/                 # D80 固件（u02 世代）
│   ├── aic8800D80X2/               # D80X2 固件（u05 世代）
│   ├── aic8800DC/                  # DC 固件（u02 世代）
│   └── aic8800_and_aic8800D80/     # 8800 + D80 混合目录（bsp 加载源，见下）
├── regulatory.db                   # cfg80211 监管域数据库
└── regulatory.db.p7s
```

### 3.1 bsp 模块的固件目录由硬编码决定

`aic8800_bsp.ko` 二进制内存在硬编码字符串：

```
/usr/lib/firmware/aic8800_sdio/aic8800_and_aic8800D80
```

即 bsp 从**混合目录** `aic8800_and_aic8800D80/` 加载固件，而非同名的单一芯片目录。该目录同时包含 8800 原版与 D80 两套固件，驱动按芯片类型自行选择。

### 3.2 fdrv 模块使用 request_firmware

`aic8800_fdrv.ko` 内存在 `request_firmware` / `release_firmware` 符号及 `rwnx_load_firmware` 等函数，固件上传走内核 firmware 框架。由于 `aicwf_dbg_level=1039` 未开启对应日志位，dmesg 中无 `request firmware` 行；驱动日志中可见 `firmware path = %s`、`Firmware Version: %s` 等格式串，未实际输出。

### 3.3 目录内容差异

同一固件名在 `aic8800D80/` 与混合目录 `aic8800_and_aic8800D80/` 中**大小不同**，内容并非副本关系：

| 文件 | aic8800D80/ | 混合目录 |
|---|---|---|
| fmacfw_8800d80_u02.bin | 321465 | 337184 |
| fw_patch_8800d80_u02.bin | 25300 | 32700 |
| fw_patch_table_8800d80_u02.bin | 984 | 1384 |
| lmacfw_rf_8800d80_u02.bin | 211623 | 263818 |
| aic_userconfig_8800d80.txt | 2448 | 2724 |

实际生效的是混合目录（bsp 硬编码路径指向）中的版本。

## 4. 板载 D80 的固件组合

混合目录中与 D80 相关的固件文件：

| 文件 | 大小 | 角色 |
|---|---|---|
| fmacfwbt_8800d80_u02.bin | 329972 | boot 阶段固件（bsp 下载） |
| fmacfw_8800d80_u02.bin | 337184 | 主固件（fdrv 上传） |
| fw_patch_8800d80_u02.bin | 32700 | 补丁 |
| fw_patch_8800d80_u02_ext0.bin | 16136 | 补丁扩展段 |
| fw_patch_table_8800d80_u02.bin | 1384 | 补丁表 |
| fw_adid_8800d80_u02.bin | 1708 | ADID 配置 |
| lmacfw_rf_8800d80_u02.bin | 263818 | LMAC/RF 固件 |

同目录内还含 D80 变体：

- `fmacfw_8800d80_h_u02.bin`（336792）与 `fmacfwbt_8800d80_h_u02.bin`（329580）：`h` 后缀高功率变体；
- 主固件与 `h` 变体的 `_ipc` 副本（同大小）；
- `fw_patch_8800d80_u04.bin`（4800）+ `fw_patch_table_8800d80_u04.bin`（416）：u04 世代补丁。

**要点：镜像内全部 D80 固件均为 u02 世代（另有 u04 补丁），不存在 u01 世代的 D80 固件。**

## 5. WiFi 功能验证

板载 iperf3 3.14（riscv64）运行 server，宿主机通过板端 AP 关联后测试（宿主机 90:de:80:92:bc:cd，信号 -26 dBm，协商速率 54 Mbit/s）：

| 方向 | 平均吞吐 | 观察 |
|---|---|---|
| 板 AP → 宿主机 | 27.7 Mbit/s | 每秒 22~43 Mbit/s 波动，8 次重传 |
| 宿主机 → 板 AP | 27.8 Mbit/s | 每秒稳定 25.0~25.3 Mbit/s，0 重传 |

吞吐上限受 54 Mbit/s 协商速率（11g 上限）约束。模块参数 `ht_on=Y` 但实际协商未进入 11n HT 速率，AP 侧速率协商行为存在疑点，后续可进一步调查。

dmesg 中 WiFi 相关关键行：

```
aicwf_sdio mmc1:390b:1 wlan0: AP started: ch=0, bcmc_idx=33 channel=2412 bw=1
aicwf_sdio mmc1:390b:1 wlan0: Add sta 0 (90:de:80:92:bc:cd) flags=[WME][AUTHENTICATED][ASSOCIATED]
```

## 6. 结论与 starry 侧关联

1. vendor Linux 镜像对板载 D80 芯片使用 **u02 主固件 + u02 补丁组合**（可选 u04 补丁），且该组合下 AP 功能与吞吐表现正常。结合此前板载芯片为 U01 revision 的读回结果，**u02 固件运行于 U01 revision 芯片是 vendor 官方支持的组合**，固件版本本身不构成 starry 侧异常的充分解释。
2. starry 侧的怀疑焦点应继续放在 **patch 配置项**（如 AMSDU_RX 等旧驱动存在、dev 重构后丢失的配置）上，而非固件文件版本不匹配。
3. 若需与 starry 侧固件比对，注意 Linux 侧实际生效的是**混合目录** `aic8800_and_aic8800D80/` 中的版本（与 `aic8800D80/` 同名文件内容不同），以 bsp.ko 硬编码路径为准。

## 7. 资产归档

本目录下的取证副本（自板端拉取）：

| 路径 | 内容 |
|---|---|
| `fw/` | `/usr/lib/firmware/` 下全部 WiFi 固件资产（aic8800_sdio 全 5 目录 + regulatory.db*），78 个文件 |
| `driver-ko/` | aic8800_bsp.ko、aic8800_fdrv.ko、aic8800_btlpm.ko（固件路径硬编码的取证对象） |
| `dmesg-wifi.log` | 取证时的板端完整 dmesg |
