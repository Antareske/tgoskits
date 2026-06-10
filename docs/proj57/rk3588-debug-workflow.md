# RK3588 StarryOS + 用户态 开发/调试工作流

本文描述在 Orange Pi 5 Plus（RK3588）上开发、调试 StarryOS 内核与用户态的完整工作流，涵盖两条互补的上板路径：

- **整盘镜像自启动**：把内核与用户态做成 TF 卡镜像，板子上电自动引导。适合稳定验收、脱机运行。
- **`loady` 串口加载**：用 tgoskits 经串口把内核直接送进板子 U-Boot 运行，不动 TF 卡内容。适合快速迭代调试。

环境与硬件准备见 `rk3588-env-prepare.md`，镜像制作细节见 `rk3588-starryos-build-manual.md`，本文不再赘述。

---

## 1. tgoskits 配置信息

Orange Pi 5 Plus 的 quick-start 由两份模板配置驱动，位于 `os/StarryOS/configs/board/`：

### 1.1 构建配置 `orangepi-5-plus.toml`

编译期配置，决定内核如何构建：

```toml
target = "aarch64-unknown-none-softfloat"   # 裸机 aarch64 目标
features = [                                # RK3588 板级驱动
  "ax-driver/list-pci-devices",
  "ax-driver/rk3588-pcie",
  "ax-driver/realtek-rtl8125",              # RTL8125 网卡
  "ax-driver/rockchip-soc",
  "ax-driver/rockchip-dwc-xhci",            # USB3
  "ax-driver/rockchip-sdhci",               # eMMC/SD
  "ax-driver/rockchip-dwmmc",               # SD/TF
  "rknpu",
]
log = "Info"                                # 日志级别
max_cpu_num = 8                             # SMP 核数（编译期固化）
plat_dyn = true                             # 动态平台
```

- `max_cpu_num`：内核的 SMP 核数是**编译期**写死的，运行时不可改。修改后需重新 build。
- 改动这些值后若之前构建过，需删除缓存模板 `tmp/axbuild/config/starryos/quick-start/orangepi-5-plus.toml`，否则沿用旧值。

### 1.2 运行配置 `orangepi-5-plus-uboot.toml`

`loady` 路径专用，由 ostool 的 U-Boot runner 消费，**不参与镜像构建**：

```toml
serial = "/dev/ttyUSB0"                     # 串口设备
baud_rate = "1500000"                       # 波特率
dtb_file = "os/StarryOS/configs/board/orangepi-5-plus.dtb"   # 传给内核的 DTB
success_regex = []                          # 命中即判定成功
fail_regex = ["panicked at"]                # 命中即判定失败并返回
```

- `serial` / `baud_rate`：也可在命令行用 `--serial` / `--baud` 覆盖，命令行优先。
- `fail_regex`：串口输出一旦匹配 `panicked at`，ostool 会判定失败退出。

---

## 2. tgoskits starry 命令

### 2.1 构建

```bash
cargo xtask starry quick-start orangepi-5-plus build
```

产出内核 ELF `target/aarch64-unknown-none-softfloat/release/starryos` 及其 `.bin`。

可选覆盖参数：`--serial`、`--baud`、`--dtb`（写入 tmp 运行配置，供后续 run 使用）。

### 2.2 下载根文件系统

```bash
cargo xtask starry rootfs --arch aarch64
```

预构建的 aarch64 ext4 根文件系统（含用户态）下载到 `tmp/axbuild/rootfs/`。

### 2.3 串口加载运行（loady）

动态加载 tgoskits 中的 starryos 内核：build 后经串口把内核直接送进板子 U-Boot 并 `bootm`，无需重做/重烧 TF 卡。

tgoskits 会重新编译 starryos 内核，打包后加载到板子内存中，不再读取 TF 卡上烧录的系统，适合改完内核代码后直接试验。注意系统仍使用 TF 卡上的文件系统。

```bash
cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0
```

**上电时机（关键）**：命令会先构建并打包，然后打开串口，输出：

```text
Waiting for board on power or reset...
```

看到这一行后，**再给板子上电（接通电源或按电源键开机）**。ostool 会接管刚上电的 U-Boot，自动完成 `loady` 上传与 `bootm`。

也可**提前让板子停在 U-Boot**（上电后串口连按空格或 Ctrl+C 打断 autoboot，停在 `=>` 提示符），再执行本命令；ostool 同样能接管已在 U-Boot 的板子。

**失败重试**：若 `Waiting for board...` 之后紧跟的 ostool 指令失败返回（如串口握手、loady 超时等），按以下顺序重来：

1. 给板子**断电**。
2. **重新执行** `cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0`。
3. 等再次出现 `Waiting for board on power or reset...` 后，**再给板子上电**重试。

要点是每次重试都让「命令进入等待」与「板子上电」的先后顺序保持一致：先等待，后上电。

---

## 3. 串口交互工具 picocom

`loady` run 跑完或镜像自启动后，若想直接和板子串口交互（进 StarryOS shell、看日志、打断 U-Boot），可用工具 picocom：

```bash
picocom -b 1500000 /dev/ttyUSB0
```

常用操作：

- 波特率固定 `1500000`，设备名以实际为准（`ls /dev/ttyUSB*`）。
- 退出：`Ctrl+a+q`。
- 同一时刻串口只能被一个程序占用。用 picocom 前确认没有其他进程（包括上一条还没退出的 ostool run）占着 `/dev/ttyUSB0`，否则会打不开或抢不到输出。
- 串口上内核日志与用户命令输出可能交错在同一行，属正常现象，以命令是否返回、退出码为准。

成功进入用户态后串口出现：

```text
Welcome to Starry OS!
root@starry:/root #
```

StarryOS 启动成功后，可向 StarryOS 上传和测试用户态程序。

---

## 4. 两条路径的选择

| 场景 | 推荐路径 | 说明 |
| --- | --- | --- |
| 频繁改内核、快速看效果 | `loady` run | 改完 `run` 直接构建 & 加载新内核 |
| 稳定验收、脱机运行、交付镜像 | 整盘镜像自启动 | 见 `rk3588-starryos-build-manual.md` |
| 只想看串口输出 / 打断 U-Boot | picocom | 独立串口终端 |

典型调试循环：

```text
改代码
  → cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0
  → 见 "Waiting for board..." 后给板子上电
  → 串口观察启动与 panic
  → (失败则断电、重跑命令、再上电)
  → 进 shell 验证用户态
```

---

## 5. 常见问题

- **`run` 卡在 `Waiting for board...`**：还没给板子上电，或上电时机早于该提示。先确认看到提示再上电。
- **`run` 紧接着报错返回**：串口被占用 / 握手失败 / 上电时机不对。断电 → 重跑命令 → 见提示后再上电。
- **串口无输出**：TTL 引脚接线、波特率（1500000）、串口是否被其他进程占用、USB 是否已映射到当前环境（见 `rk3588-env-prepare.md`）。
