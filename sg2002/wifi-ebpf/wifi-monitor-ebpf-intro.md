wifi_monitor — SG2002 WiFi 发送路径 eBPF 探针¶
分支：sg2002/wifi-ebpf

路径：apps/starry/ebpf/wifi_monitor

功能¶
TX 帧计数¶
统计每次 WiFi 帧入队的帧数和以太网字节数。

SDIO 写入字节与错误计数¶
统计每次 SDIO write_fifo 提交的字节数，以及写入失败的次数。

SDIO 写延迟直方图¶
记录每次 write_fifo 从进入到返回的耗时，按 8 个延迟桶分别计数。桶界（μs）：<300, <500, <1k, <5k, <20k, <50k, <100k, ≥100k。

SDHCI 中断次数¶
统计 sdhci_irq_handler 被调用的次数。

输出¶
用户态 loader 每 5 秒打印一轮计数器和直方图：

=== wifi_monitor ===
TX_ENQUEUE  cnt=...  bytes=...
SDIO_WR      bytes=...  err=...
IRQ          cnt=...
SDIO_WR_LAT  <300us  <500us  <1ms  <5ms  <20ms  <50ms  <100ms  >=100ms  (total=...)

构建¶
prebuild.sh 交叉编译 riscv64-linux-musl 静态二进制，install 到 StarryOS rootfs overlay 的 /usr/bin/wifi_monitor。eBPF 字节码嵌入 loader 二进制，无额外运行时依赖。

限制¶
WR_ENTRY 只有一个槽，write_fifo 有多线程调用方（TX / WPA2 握手 / 启动），并发交错时入口时间戳或长度可能被覆写。
