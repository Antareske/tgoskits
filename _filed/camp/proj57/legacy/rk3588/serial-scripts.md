串口辅助脚本命令如下，默认串口都是 /dev/ttyUSB0。

# 测试命令脚本

cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0

## 启动 StarryOS quick-start

./scripts/orangepi-run.sh

等价于：

cargo xtask starry quick-start orangepi-5-plus run --serial /dev/ttyUSB0

## 打开串口控制台，默认 1500000 baud

picocom -b 1500000 /dev/ttyUSB0

./scripts/serial-console.sh /dev/ttyUSB0

或显式波特率：

./scripts/serial-console.sh /dev/ttyUSB0 1500000

## 持续发送 Ctrl-C，用来打断 U-Boot autoboot，停在 U-Boot shell

./scripts/orangepi-uboot-hold.sh

## 向当前串口 shell 发送 reboot

./scripts/orangepi-reboot.sh

## 同上，别名式脚本

./scripts/orangepi-serial-reboot.sh

## 向当前串口 shell 发送 poweroff

./scripts/orangepi-poweroff.sh

如果串口设备不是 /dev/ttyUSB0，可以这样覆盖：

SERIAL=/dev/ttyUSB1 ./scripts/orangepi-run.sh

SERIAL=/dev/ttyUSB1 ./scripts/orangepi-uboot-hold.sh

SERIAL=/dev/ttyUSB1 ./scripts/orangepi-reboot.sh

SERIAL=/dev/ttyUSB1 ./scripts/orangepi-poweroff.sh
