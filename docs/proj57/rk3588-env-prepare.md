# RK3588 开发环境准备

本文说明在 Orange Pi 5 Plus（RK3588）上开发、调试 StarryOS 前的环境、串口连接与镜像烧录准备。本文将镜像视为已事先准备好的资产，镜像制作见 `rk3588-starryos-build-manual.md`，开发/调试工作流见 `rk3588-debug-workflow.md`。

---

## 1. 环境与硬件

软件环境（自底向上）：

- Windows PC（宿主）
- WSL
- Docker Desktop
- 开发容器（tgoskits）

硬件准备：

- RK3588 开发板 + 电源
- TF 卡 + 读卡器
- CH340 USB 转 TTL 模块（需支持 1500000 波特率）

其它参考说明：

- `OrangePi_5_Plus_RK3588_用户手册_v2.1`，见香橙派官网。

---

## 2. tgoskits 开发容器

开发容器配置方法参考：https://github.com/rcore-os/tgoskits/pull/252

> 容器默认不透传宿主机 USB 串口设备，需额外修改开发容器配置以支持透传 WSL 的 USB 串口。使用对应分支的开发容器配置 `./.devcontainer/devcontainer.json`。

---

## 3. 串口连接开发板

串口波特率统一为 **1500000**。根据读取串口的位置，分两种方式。

### 3.1 在 Windows 主机直接登录

用 CH340 模块连接开发板与 Windows PC，通过 MobaXterm 读取开发板输出：

```text
MobaXterm → Session → Serial → 选择 CH340 的 port，波特率 1500000
```

### 3.2 在 WSL / 开发容器里登录

开发板上电后，在 Linux 开发环境里读取开发板输出时，需要先用 `usbipd` 把 CH340 的 USB 设备从 Windows 映射进 WSL / 容器。

以下命令在 **PowerShell（管理员）** 中运行。

安装 usbipd-win（一次即可）：

```powershell
winget install usbipd
```

列出设备（插入 Orange Pi 串口线后，找到 USB 串口设备，如 `1-3`）：

```powershell
usbipd list
```

绑定（持久化，可能需要 `--force` 或重启电脑）：

```powershell
usbipd bind --busid 1-3
# 或
usbipd bind --busid 1-3 --force
```

附加到 WSL，使虚拟机对其可见（附加时需 WSL 已开机、开发容器未开机）：

```powershell
usbipd attach --wsl --busid 1-3 --auto-attach
```

解除映射（让 Windows / MobaXterm 重新可用）：

```powershell
usbipd detach --busid 1-3
```

解绑（完全回退驱动）：

```powershell
usbipd unbind --busid 1-3
```

`usbipd attach` 后，该 USB 串口会在所有已启动的 WSL 中映射为 `/dev/ttyUSBX`（如 `/dev/ttyUSB0`），此时即可在 WSL 中读写该串口。

> **在开发容器中使用串口**：先启动 Docker Desktop，再执行 `usbipd attach`，保证容器开机时能读到其宿主（WSL）上的设备。注意容器创建时需额外配置参数才能透传宿主机设备。

---

## 4. 烧录镜像到 TF 卡

在 Windows 宿主机上使用 [balenaEtcher](https://etcher.balena.io/) 把镜像写入 TF 卡。两类镜像的烧录方式相同，区别只在所选镜像文件：

1. 将 TF 卡插入读卡器并接入 Windows PC。
2. 打开 balenaEtcher，依次操作：
   - **Flash from file**：选择镜像文件。
   - **Select target**：选择 TF 卡对应的设备（注意核对容量，避免误选其他磁盘）。
   - **Flash!**：开始烧录，完成后 Etcher 会自动校验。

balenaEtcher 可直接处理 `.img` 以及 `.img.xz` / `.img.gz` 等压缩镜像，一般无需手动解压。

### 4.1 烧录 Linux 镜像（如 香橙派官方镜像、Armbian 等）

用于在板子上运行通用 Linux，或作为构建启动链 / 设备树的来源镜像。直接选择官方 Linux 整盘镜像烧录即可。

香橙派官方 http://www.orangepi.cn/html/hardWare/computerAndMicrocontrollers/service-and-support/Orange-Pi-5-plus.html

Armbian https://armbian.com/boards/orangepi5-plus

### 4.2 烧录准备好的 Starry 镜像

用于在板子上自启动纯 StarryOS。选择事先准备好的 Starry 整盘镜像烧录即可。

> 烧录完成后即可把 TF 卡插入开发板上电启动，启动与串口观察见 `rk3588-debug-workflow.md`。

---

## 5. 问题排查

**镜像烧录不成功**

- 换用正规厂商的 TF 卡
- 换用良好的读卡器
- 手动解压被压缩的镜像后再烧录

**板子上电后串口无输出**

- 确认 TTL-USB 引脚接线正确（GND / RX / TX，板子 TX 接 USB RX、板子 RX 接 USB TX）
- 确认没有其他程序/进程占用串口
- 确认串口已正确映射到主机 / 虚拟机
- 减少开发板上插接的外设
- 检查开发板供电电压是否过低或过高
