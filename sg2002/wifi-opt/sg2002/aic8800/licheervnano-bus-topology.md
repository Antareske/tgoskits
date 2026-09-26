# SG2002 荔枝派 Nano 总线拓扑调查

**调查对象**：SG2002 荔枝派 Nano（LicheeRV Nano）的 SDIO / 蓝牙 / USB 走向
**结论日期**：2026-09-12
**证据来源**：

- 源码：`sipeed/LicheeRV-Nano-Build` @ `d4003f15b`（本机副本 `/home/asta/work/LicheeRV-Nano-Build`）
- 运行时：官方镜像 `2026-01-14-16-03-d4003f.img`（FAT@扇区1，ext4@扇区32769），其中 `boot.sd` 是 FIT 镜像，内含板级 DTB，已抽出反编译为 `/tmp/board.dts`
- 二进制：镜像内 `/mnt/system/ko/` 下的 `.ko` 与实际 init 脚本

引用格式：`文件:行号`，相对仓库根或镜像根。**每个结论都标注置信度**（见文末分级说明）。

---

## 0. 通俗结论

板子上有一颗 WiFi+蓝牙二合一无线模块（Sipeed 不同批次可能是 AIC8800 或 Realtek 8733BS）。

- **SDIO 那条线是无线模块专享的**：SoC 里实际有**两个**互相独立的 SD 控制器（可以想成两部独立电梯），一个接 microSD 卡槽用于启动，一个接无线模块；两个都被硬件设计"封条"限定了用途，谁也抢不走谁的活。要修正的是说法本身：不是"全芯片唯一一个 SD 控制器被它独占"，而是"它独占了两个中的一个"。
- **蓝牙不走串口，蹭的是 WiFi 那条 SDIO 线**：蓝牙数据被打包塞进 WiFi 链路里传（芯片内部隧道机制），厂商驱动的编译配置和镜像里的二进制都能证明。板子上确实把 UART1 四根线（收发+两路流控）复用出来并注释成"蓝牙串口"（**是否真的走到模组引脚，仓库里无从证明**），**但整个官方系统没有任何程序去用它** —— 相当于墙里预埋了管子、两头开口，但没接水管。
- **USB 就是 USB，不是串口**：板上 Type-C 接的是 SoC 自带的 USB 2.0 OTG 控制器。出厂被配成"从设备 + USB 网卡"，插到电脑上是多一个网卡，不是串口。

---

## 1. SDIO 总线与 SDHCI

### 1.1 芯片里有两个独立 SDHCI 实例【确证】

| | 控制器 A | 控制器 B |
|---|---|---|
| DT 节点 | `cv-sd@4310000` | `wifi-sd@4320000` |
| compatible | `cvitek,cv181x-sd` | `cvitek,cv181x-sdio` |
| 中断 | 36 | 38 |
| PLL | `pll_index=6` / `pll_reg=0x3002070` | `pll_index=7` / `pll_reg=0x300207C` |
| 用途封条 | **`no-sdio`**（禁止 SDIO 设备）、`no-mmc` | **`non-removable`**、**`no-sd`**、**`no-mmc`** |
| 用途 | microSD 启动卡（有 `cvi-cd-gpios` 卡检测） | 无线模块 |

证据：

- `build/boards/default/dts/sg200x/soph_base.dtsi:500-523`（控制器 A）、`:525-541`（控制器 B）
- `build/boards/default/dts/sg200x/soph_base_riscv.dtsi:281-284`（A 的中断）、`:332-335`（B 的中断）
- 两者由**同一份驱动**匹配：`linux_5.10/drivers/mmc/host/cvitek/sdhci-cv181x.c:1158-1162` 同时声明 `cvitek,cv181x-sd` 与 `cvitek,cv181x-sdio`；`build/boards/sg200x/sg2002_licheervnano_sd/linux/sg2002_licheervnano_sd_defconfig:384-387` `CONFIG_MMC_SDHCI_CVI=y`
- 运行时 DTB：`/tmp/board.dts:624-649`（A，IRQ `0x24`=36）、`:651-669`（B，IRQ `0x26`=38，`status="okay"`）

**QFN 封装的一个特殊之处**【确证】：芯片级 DTS 模板里 SDIO 节点是被删掉的（`build/boards/default/dts/sg200x/soph_asic_qfn.dtsi:105` `/delete-node/ wifi-sd@4320000;`，同处共 6 项删除，其余 5 项为 i2s/sound），由板级 DTS 重新声明（`build/boards/sg200x/sg2002_licheervnano_sd/dts_riscv/sg2002_licheervnano_sd.dts:244-271`，板级把频率从模板的 50 MHz 降到 25 MHz）。**判断某个外设在 QFN 上是否可用时不能只看芯片级 delete-node，要看板级是否重新声明。**

### 1.2 "独占"成立，但独占者是"无线模块"而非某个驱动【确证】

全 SG2002 设备树里 `cvitek,cv181x-sdio` 只有这一个节点（全树 grep 确认），且它 `non-removable` + `no-sd` + `no-mmc` → 只能挂 SDIO 从设备，不可能被存储卡占用。

但官方镜像同时加载**两套 SDIO 无线驱动**（`buildroot/board/cvitek/SG200X/overlay/etc/init.d/S25wifimod:9-12`，镜像内 `/etc/init.d/S25wifimod` 一致）：

```
insmod 3rd/aic8800_bsp.ko      # AIC8800 WiFi
insmod 3rd/aic8800_fdrv.ko
insmod 3rd/8733bs.ko           # Realtek RTL8733BS SDIO WiFi
```

原因是 Sipeed 不同批次焊不同模组（`osdrv/extdrv/wireless/rtl8733bs/Kconfig:3` 明写 "Realtek 8733B SDIO WiFi"）。**哪颗在板上，哪颗 probe 成功**，另一颗注册失败但无害。

### 1.3 AIC8800 在总线上占两个 function，蓝牙不是独立 function【确证】

- 卡的 SDIO 匹配表只认 **WLAN class**：`osdrv/extdrv/wireless/aic8800/aic8800_bsp/aicsdio.c:466-469` `{SDIO_DEVICE_CLASS(SDIO_CLASS_WLAN)}`；整个 aic8800 目录**没有任何 `SDIO_CLASS_BT_*`**。
- fdrv 只接受 **function 1**：`aic8800_fdrv/aicwf_sdio.c:777` `if (func->num != 1) return err;`
- bsp 只接受 **function 2**，随后自己换成 function 1 使用：`aic8800_bsp/aicsdio.c:295`（`if (func->num != 2) return err;`）、`:302`（`func = func->card->sdio_func[1 - 1]; //replace 2 with 1`）；对 DC/DW 变体把 func2 留作 `func_msg`（`:325`），只用于 wakeup 寄存器与块大小。

**对照**：Realtek 模块的蓝牙伴侣驱动是标准 BT class 的独立 SDIO function —— `osdrv/extdrv/wireless/rtl8733bs_bt_sdio/btrtk_sdio.c:74-80` 匹配 `SDIO_CLASS_BT_A/B/AMP`。这反证了 AIC 方案不是"BT 作为 SDIO function 枚举"。

---

## 2. 蓝牙：软件栈走 SDIO，UART1 只是硬件留线

### 2.1 驱动编译配置选择 SDIO-BT【确证】

| 证据 | 内容 |
|---|---|
| `osdrv/extdrv/wireless/aic8800/aic8800_fdrv/Makefile:76` | `CONFIG_SDIO_BT=y`（另有 `CONFIG_USB_BT=n`、`CONFIG_USB_SUPPORT=n`） |
| `.../aic8800_fdrv/Makefile:209-210` | `$(MODULE_NAME)-$(CONFIG_SDIO_BT) += aic_btsdio.o` / `+= btsdio.o` |
| `.../aic8800_fdrv/aic_btsdio.h:21-24` | `#ifdef CONFIG_PLATFORM_UBUNTU → #define CONFIG_BLUEDROID 0`（BlueZ 分支）；顶层 Makefile 里 `export CONFIG_PLATFORM_UBUNTU = y` |
| `.../aic8800_fdrv/rwnx_main.c:6045-6052` | 接口创建时：`#ifdef CONFIG_SDIO_BT` → `#if CONFIG_BLUEDROID` 走 `btchr_init()/hdev_init()`，`#else` 走 **`btsdio_init()`** |
| `.../aic8800_bsp/Makefile:34` | `CONFIG_SDIO_BT = y`（硬赋值，不随平台变） |
| `.../aic8800_bsp/aic_bsp_main.c:29-36` | 该宏成立时 `wl_fw = "fmacfwbt.bin"`（BT 组合固件），否则 `fmacfw.bin` |

即：`insmod aic8800_fdrv.ko` 成功后，驱动会 `hci_register_dev()` 注册一个 **bus = HCI_SDIO** 的 HCI 设备（`btsdio.c`，BlueZ 分支），HCI 数据经 LMAC 消息 `TDLS_SDIO_BT_SEND_REQ` 隧道传输；BT 核的 patch 固件由 bsp **经 SDIO 写入**（`aic_bsp_driver.c` 的 `aicbt_init` / `aicbt_patch_trap_data_load`，先判 `btenable`）。

### 2.2 镜像里的二进制也这么说【确证】

直接 `strings` 检验镜像内模块：

- `aic8800_fdrv.ko`：含 `aic_btsdio`、`btsdio_open/close/flush/tx_packet`、`btsdio_work`、`Failed to get hci dev` 等符号（19 处 `btsdio` 命中）
- `aic8800_bsp.ko`：引用 `fmacfwbt.bin`、`fmacfwbt_8800d80_u02.bin`、`fmacfwbt_8800d80_h_u02.bin`、`fmacfw_*_hbt_u02.*`、`fw_adid.bin`、`fw_patch.bin`、`fw_patch_table.bin`，且固件根路径为 `/usr/lib/firmware/aic8800_sdio/aic8800_and_aic8800D80`（与 `osdrv/extdrv/wireless/aic8800/Kconfig:7-13` 一致）
- 镜像固件目录中确实存在 `fmacfwbt.bin` 及其各变体

### 2.3 UART1 侧：硬件留了线，软件没人用【确证（软件侧）/ 见 2.5（硬件侧）】

u-boot 板级初始化里有一段完整注释为 `// uart bluetooth` 的 `build/boards/sg200x/sg2002_licheervnano_sd/u-boot/cvi_board_init.c:99-103`：

```
mmio_write_32(0x03001070, 0x1); // GPIOA 28 UART1 TX
mmio_write_32(0x03001074, 0x1); // GPIOA 29 UART1 RX
mmio_write_32(0x03001068, 0x4); // GPIOA 18 UART1 CTS
mmio_write_32(0x03001064, 0x4); // GPIOA 19 UART1 RTS
```

这段不是过时注释，与 SoC 引脚表逐条对得上（`fsbl/plat/cv180x/include/cv180x_pinlist_swconfig.h`）：

- `:109/:116` `IIC0_SCL__UART1_TX 1` / `IIC0_SDA__UART1_RX 1` —— 而 pad 0x03001070/0x03001074 正是 IIC0_SCL/SDA（XGPIOA28/29，见 `build/boards/default/dts/sg200x/soph_asic_qfn.dtsi:61-62`）
- `:105` `JTAG_CPU_TCK__UART1_CTS 4`（该 pad 即 XGPIOA18）、`:98` `JTAG_CPU_TMS__UART1_RTS 4`（即 XGPIOA19）

即 **UART1 的收发与两路流控都被真的复用出来了，带硬件流控**。

但软件层没有任何消费者：

- 全镜像 grep：**没有任何 `hciattach` / `btattach` / `brcm_patchram` 调用**（工具二进制在，`/usr/bin/hciattach`、`/usr/bin/btattach` 是 bluez 包带的，没人调）
- `S25wifimod` 不加载任何 BT 模块；`/mnt/system/ko/3rd/aic8800_btlpm.ko` 和 `btrtksdio.ko` **在目录里躺着但从不 insmod**
- `aic8800_btlpm.ko` 自身是 UART/bluesleep 风格的低功耗模块：字符串含 `bluesleep_uart_dev`、`bluesleep_get_uart_port`、`hsuart_power`、`uart_index`、`uart%d`，属其他平台（Android/bluesleep）方案，本板未启用
- 内核 defconfig 保留 UART-BT 的能力但未用：`build/boards/sg200x/sg2002_licheervnano_sd/linux/sg2002_licheervnano_sd_defconfig:508-520` 有 `CONFIG_BT_HCIUART=y` + H4/3WIRE/BCSP/RTL，**没有 `CONFIG_BT_HCIBTSDIO`**
- 设备树里没有任何 `bluetooth {}` 子节点 / hci 节点（全树 grep 仅命中两处 u-boot 注释）

### 2.4 官方镜像里蓝牙的实际状态【确证】

- `bluetoothd` 会由 `S40bluetoothd`（buildroot 标准脚本）拉起，bluez 工具齐全
- 蓝牙 HCI 设备只可能来自 AIC fdrv 注册的 SDIO 设备（代码路径确证；未上板观测）；**没有任何针对 UART 的初始化**
- 因此"蓝牙走 UART"在这份镜像里不成立

### 2.5 未确证项：模块的蓝牙物理接线

**仓库里没有原理图**，无法证明模组的 BT 引脚物理上接的是 UART1 还是内部隧道。已知事实是：u-boot 把 UART1（含流控）复用出来了，而厂商的 Linux 驱动选择走 SDIO 隧道 —— 芯片两种都支持，两者不矛盾，但**哪条是这块板的实际物理通路，必须上板或看原理图确认**。

上板验证方法（都可以在官方镜像上直接跑）：

```sh
# 1) 看蓝牙设备从哪来：Bus 显示 SDIO 还是 UART
dmesg | grep -iE "btsdio|aic|hci"
hciconfig -a            # 或 cat /sys/class/bluetooth/*/bus

# 2) 看 SDIO 卡上枚举了几个 function
ls /sys/bus/sdio/devices/
cat /sys/bus/sdio/devices/*/vendor /sys/bus/sdio/devices/*/device
#   c8a1:0082 = AIC8800D80，c8a1:c08d = AIC8800DC（见 aicsdio.c:78,83-84）

# 3) 反向测 UART 那条路（若模块 BT 真是 UART，这里才会有反应）
ls /dev/ttyS1
btattach -B /dev/ttyS1 -S 115200
```

---

## 3. USB：SoC 原生 OTG，不是串口

### 3.1 硬件存在且使能【确证】

- `build/boards/default/dts/sg200x/soph_base.dtsi:833-850`：`usb: usb@04340000`，`compatible = "cvitek,cv182x-usb"`，reg 含控制器 `0x04340000` 与 `0x03006000`（注释写明 `//USB 2.0 PHY`），`dr_mode = "otg"`，`vbus-gpio`，`status = "okay"`
- `build/boards/default/dts/sg200x/soph_base_riscv.dtsi:362-365`：中断 30
- 内核：`.../linux/sg2002_licheervnano_sd_defconfig:381-383` `CONFIG_USB_DWC2=y` / `CONFIG_USB_GADGET=y`
- QFN 封装**没有**裁掉 USB（`soph_asic_qfn.dtsi:104-109` 的删除清单里没有 usb）
- 运行时 DTB `/tmp/board.dts:918-932`：`status="okay"`、`dr_mode="otg"`

### 3.2 出厂是"从设备 + USB 网卡"【确证】

- 镜像 FAT 分区里带三个标记文件：`usb.dev`、`usb.ncm`、`usb.rndis`（镜像 `boot/` 目录实测）
- 镜像内 `/etc/init.d/S08usbdev`：`/boot/usb.host` → `echo host > /proc/cviusb/otg_role`；`/boot/usb.dev` → 建 configfs gadget，并按 `usb.ncm`/`usb.rndis`/`usb.GS0`/`usb.disk0`/`usb.uvc` 等标记文件逐项挂功能
- `S30gadget_nic` 只在 `usb.dev` 存在时启动，把 RNDIS/NCM 网口起成 DHCP 服务
- **想让它当 USB 主机**（接 U 盘/网卡/摄像头）：`touch /boot/usb.host` 后重启，或 `echo host > /proc/cviusb/otg_role`
- 镜像 `/mnt/system/ko/` 里带了 USB **主机侧**驱动：`asix.ko`、`ax88179_178a.ko`、`cdc_ether.ko`、`cdc_ncm.ko`、`rndis_host.ko`、`rndis_wlan.ko`、`cp210x.ko` —— 佐证这个口双向可用

### 3.3 "USB 走串口"说法的可能来源【推断】

两个现象容易混淆：

1. **串口控制台在 UART0**，板上是排针，要自行外接 USB-TTL 模块才能看。u-boot bootargs 为 `console=ttyS0,115200`（`u-boot-2021.10/include/configs/phobos-asic.h:335-338`），运行时 DTB `chosen { stdout-path = "serial0"; }`（`/tmp/board.dts:1000-1002`）
2. **USB 口可以枚举成 CDC-ACM 串口**：gadget 支持 `acm.GS0`（需 `/boot/usb.GS0` 标记，出厂**未开**），且 overlay inittab 里预先写了一条 `ttyGS0` 的 getty（`buildroot/board/cvitek/SG200X/overlay/etc/inittab:21`，设备不存在时 respawn 失败无害）；`demo/tioccons.c` 还能用 `TIOCCONS` 把内核 console 重定向到该 tty

但即使枚举成串口，那是 **SoC USB 控制器虚拟出来的假串口（USB CDC-ACM）**，与 UART 无关。

### 3.4 一处与本文结论无关但值得记录的出入【确证】

镜像 `/mnt/system/ko/` 里的部分模块（如 `asix.ko`、`cp210x.ko`）在本板 defconfig 里**找不到对应配置项**（`sg2002_licheervnano_sd_defconfig:93` 甚至写着 `# CONFIG_USB_NET_DRIVERS is not set`；在 `build/` 目录与本板 defconfig 下搜不到 `USB_SERIAL_CP210X` 等符号，只在无关板子的配置与内核源码树的 Kconfig/Makefile 里出现该符号名）。说明该目录的模块集合不完全由本仓库、本 commit 的 defconfig 决定（可能来自 SDK 预置的组合）。**这不影响本文结论** —— 本文关于 USB 的判断只依赖"镜像里有什么"，已直接核对。

---

## 4. 附带发现：SPI2 与 WiFi SDIO 复用同一批 pad【确证（复用关系）/ 推断（具体 pad 名）】

u-boot 板级初始化里，`// wifi sdio pinmux` 与紧随其后被注释掉的 `// spi2 pinmux` 写的是**同一组 6 个寄存器**（`build/boards/sg200x/sg2002_licheervnano_sd/u-boot/cvi_board_init.c:83-95`）：

```
mmio_write_32(0x030010D0, 0x0); // D3      // 注释掉的 spi2 版本写 0x1：CS
mmio_write_32(0x030010D4, 0x0); // D2
mmio_write_32(0x030010D8, 0x0); // D1      //  → DC
mmio_write_32(0x030010DC, 0x0); // D0      //  → MISO
mmio_write_32(0x030010E0, 0x0); // CMD     //  → MOSI
mmio_write_32(0x030010E4, 0x0); // CLK     //  → SCK
```

SoC 引脚表印证了这种复用关系（`fsbl/plat/cv180x/include/cv180x_pinlist_swconfig.h`）：`:176-183` `SD1_D3__SPI2_CS_X 1`、`:216-219` `SD1_CLK__SPI2_SCK 1` 等。（该引脚表来自 cv180x 目录，sg200x 没有对应文件；pad 偏移的逐条对应关系由 u-boot 注释推定。）

**当前不会冲突，但会误导**：

- 运行时 DTB 里 `spi2@041A0000` 是 `status = "okay"`，且挂着能绑定 spidev 驱动的 `spidev@0`（`compatible = "rohm,dh2228fv"`）—— 见 `/tmp/board.dts:265-281`，板级源在 `sg2002_licheervnano_sd.dts:146-165`（st7789 子节点是 `status = "disabled"`）
- 内核这边用的是通用 `spi-dw-*` 驱动（节点 compatible `snps,dw-apb-ssi`），**不碰 pad 复用**（`linux_5.10/drivers/spi/spi-dw-mmio.c`、`spi-dw-core.c`、`spi-dw-dma.c` 中无 pinmux/pad 寄存器代码；节点也没有 `pinctrl-*` 属性，`bias-pull-up` 无消费者）
- u-boot 把 pad 留在 SDIO 功能上 → **WiFi 正常，SPI2 实际是哑的**（`/dev/spidev2.0` 存在但接什么都没反应）【推断】

**结论**：试图用 SPI2（例如接 SPI 屏）会与 WiFi SDIO 争用同一批物理引脚。做 SG2002 外设扩展前必须先确认目标板是否复用这批 pad。

---

## 5. 置信度分级

| 等级 | 含义 | 本文中的条目 |
|---|---|---|
| **确证** | 有源码行号或实测（镜像/二进制）直接支撑 | SDHCI 两实例与封条属性、SDIO 独占、AIC8800 双 function 与 BT 非独立 function、驱动编译期选择 SDIO-BT、镜像二进制含 btsdio/fmacfwbt、UART1 留线无软件使用、无 hciattach 调用、USB 为原生 dwc2 OTG、出厂 gadget 配置、SPI2 与 SDIO 寄存器复用 |
| **推断** | 由确证事实推出，但未直接观测 | "SPI2 实际是哑的"、"USB 走串口"说法的来源、pad 偏移的逐条对应（引脚表来自 cv180x） |
| **未确证** | 仓库内无证据，需上板或原理图 | 模组蓝牙的**物理**接线（UART 还是 SDIO 隧道）、`vbus-gpio` 的实际电平检测行为 |

---

## 6. 复现方法

```sh
# 取源码
git clone --depth 1 git@github.com:sipeed/LicheeRV-Nano-Build.git   # 本机 GitHub 只通 SSH

# 从官方镜像取"实际运行"的设备树（比读源码可靠：能反映 asic 层 delete-node 与板级覆盖的结果）
sudo losetup -f --show -P 2026-01-14-16-03-d4003f.img     # → /dev/loopN
sudo mount -o ro /dev/loopNp1 /mnt/boot && sudo mount -o ro /dev/loopNp2 /mnt/root
sudo grep -abo $'\xd0\x0d\xfe\xed' /mnt/boot/boot.sd      # 第二个魔数偏移即板级 DTB
sudo dd if=/mnt/boot/boot.sd of=board.dtb bs=1 skip=<偏移>
dtc -I dtb -O dts -o board.dts board.dtb
sudo umount /mnt/boot /mnt/root && sudo losetup -D

# 查模块（读 .ko 二进制，不依赖上板）
strings /mnt/root/mnt/system/ko/3rd/aic8800_fdrv.ko | grep -i btsdio
strings /mnt/root/mnt/system/ko/3rd/aic8800_bsp.ko  | grep -E "fmacfwbt|^fw_adid"
```

（`boot.sd` 本身就是 FIT 镜像，因为 `build/boards/sg200x/sg2002_licheervnano_sd/sg2002_licheervnano_sd_defconfig` 里 `CONFIG_BOOT_IMAGE_SINGLE_DTB=y`；文件里第一个 DTB 魔数是 FIT 自己的头，板级 DTB 嵌在 `images/fdt-sg2002_licheervnano_sd` 节点。）

---

## 7. 对 StarryOS（tgoskits）SG2002 工作的影响

1. **驱动假设可以简化**：SDIO 实例被无线模块独占，可以放心按"整条 SDIO 总线归一颗组合芯片"设计，不必为 SD 卡共线做仲裁（SD 卡在另一个控制器上）。
2. **蓝牙与 WiFi 是同一条链路上的两个消费者**：BT 数据经 SDIO 隧道（LMAC 消息）走，若日后要支持 BT，需要在该链路上复用已完成的消息收发路径，而不是另起 UART 驱动；`aic8800` crate 里已有 `bluetooth_enabled`（bit 26）与 HBT 固件选择（`drivers/net/aic8800/src/device/startup/dc.rs:148,166,192`），方向一致。
3. **UART1 不要当作 BT 已接线来用**：软件栈从未启用，物理接线未确证。
4. **片外扩展注意 pad 复用**：SPI2 与 WiFi SDIO 争 pin（见 §4）。

---

*本文所有"确证"条目均可按 §6 复现。若与上游新版本不符，以复现结果为准。*
