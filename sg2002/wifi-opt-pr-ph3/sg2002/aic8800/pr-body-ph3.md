# perf(aic8800): aggregate transmit writes and cache firmware credits

## 背景与问题

`aic8800` 的发送路径此前是「一个数据包一笔 CMD53，每笔之前读一次固件 flow-control 寄存器，一笔结束等网络运行时再次驱动才成形下一笔」。改动前在实体板卡上按 TCP、板端作 server、主机作 client、单流 `-P 1`、30 s 的口径实测：上行（板→PC）7.30 Mbits/sec、下行（PC→板）20.3 Mbits/sec。三处固定成本叠加在每一笔写上——每包一次 SDIO 命令往返还、每包一次寄存器读、以及写完到下一笔之间的 owner 空转。固件本身把一次写当帧流遍历（逐帧按声明长度步进、读到零长度结束），因此一笔写可以天然携带多帧；驱动此前没有利用这一点。

同时，发送路径有几处边界没有约束：聚合参数没有非零校验，帧长在 12-bit 声明字段之外会被静默截断，聚合写被中止或停机时其持有的 token 缺少明确的归还路径。

## 方案

**1. 一笔 CMD53 携带多帧。** 写内容按固件遍历方式布局：每帧只按发送对齐补齐，整笔写只在末尾补到 SDIO block。编码函数返回帧在流内的长度，聚合器直接使用该长度而不再从已编码字节反解声明长度；帧长放不进 12-bit 字段时编码直接失败。一笔写的帧数上限与「追加前检查的字节软目标」由 `TxAggregation` 承载，最近一次 flow-control 读数还会按固件可用 packet buffer 数收窄帧数上限。写按当前已排队帧立即成形，不等待后续帧。

**2. 固件 TX credit 本地缓存。** 一次读数在命令保留量之上时缓存下来，之后每完成一个写入 packet 扣一；命令流量与取消会让读数失效。读数高于保留量但低于按策略算出的批次阈值时，最多等待 300 µs × 8 次后重读，超预算就用当前 credit 发出——避免为了凑大批次把薄池拖成无限等待。

**3. 写完成直接续接。** 写完成时若队列里已有帧且缓存 credit 仍允许写，就直接成形并排队下一笔，不再等运行时回来；每轮 owner 推进最多续接一次，把机会让给接收扫描与 mailbox。

**4. 能力声明按芯片档位化。** `ME_CONFIG` 从固定字节改为按档位编码：DC 沿用原保守档（逐字节不变），D80 声明 40 MHz、MCS 0-9、VHT/HE、A-MSDU 上限与 PPE 阈值。档位挂在芯片 profile 上，与支持表同源。

**5. 数据帧确认标记拆分。** 普通数据帧 `hostdesc.hostid` 置 0，只有控制口帧请求固件确认，避免给每个数据包都要求一次确认。

**6. 板级聚合策略。** LicheeRV Nano 的板级设备树声明 32 帧 / 49152 字节；未声明该属性的板子保留驱动默认值。

**7. 随改动一并修正的边界。** 聚合零值在三层（核心 setter、可移植适配层选项、设备树解析）显式报错而不是静默修正；设备树只覆盖显式写出的属性；聚合写在中止、停机与未知 completion id 下把 token 恰好归还一次；事件队列满时完成事件暂存而不挤掉接收帧，也不失败设备。

## 改动点

| 位置 | 内容 |
| --- | --- |
| `drivers/net/aic8800/src/device/{data_plane,owner,progress,mailbox,startup}` | 聚合发送、credit 缓存与薄池等待、写续接、token 归还、启动冲突诊断 |
| `drivers/net/aic8800/src/{tx,protocol,lmac,profile,device/model}` | 帧流编码与长度校验、`TxAggregation` 策略、消息分类、`ME_CONFIG` 档位 |
| `drivers/net/aic8800/src/rdif/**` | 适配层的批量取帧、有界输出与完成归还、选项校验 |
| `drivers/net/aic8800/src/rdif_test_support.rs` | 新增：库内 host 测试夹具（DMA 缓冲），仅供测试构建 |
| `drivers/ax-driver/src/net/aic8800/fdt.rs` | 设备树聚合属性解析与校验 |
| `drivers/net/aic8800/{Cargo.toml,README.md}` | 测试依赖与测试布局说明 |
| `docs/design/unified-sdio-aic8800.md` | 发送帧流、聚合策略、等待与续接的契约描述 |
| `os/StarryOS/configs/board/licheerv-nano-sg2002.dtb` | 板级聚合策略属性 |

## 验证

格式化、静态检查与标准库测试均取最终一轮的结果。

### 测试结果

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 无差异 |
| `cargo xtask clippy --package aic8800` | 3/3 组合通过（base、`rdif`、`host-test`） |
| `cargo xtask clippy --package ax-driver` | 54/54 组合通过，含 `aic8800-wifi` |
| `cargo xtask test --since dev` | 选中 15 个受影响软件包，全部通过（含 `aic8800` 的 `host-test+rdif` 档案） |
| `cargo xtask test` | 全量 71 个软件包通过 |

新增的测试覆盖：一笔写携带多帧时每帧的声明长度与整笔块对齐、软字节目标越界一帧后停止增长、薄池等待的预算耗尽路径、reserve 读数不缓存、生命周期写不携带用户包、聚合 completion 含未知 id 时其余 buffer 照常归还、完成环满时以真实队列与真实 DMA 缓冲验证「暂存—归还」序列、停机与单帧输入满队时 token 恰好归还一次、帧长超出声明字段时编码失败。

### 实板测试结果

实体板卡（SG2002，LicheeRv Nano）上以板级配置构建的镜像启动：内核经 SD 卡 rootfs 启动到登录 shell，`wlan0` 完成关联并取得地址，随后板端作 iperf3 server、主机作 client 依次跑下行、上行（`-R`）与双向（`--bidir`）各若干轮。下表取每个方向的最好一轮，数字是 iperf3 自己的汇总行（整轮区间累计量）。

| 方向 | 时长 | 传输量 | 速率 |
| --- | --- | --- | --- |
| 下行（主机→板） | 120 s | 503 MBytes | 35.2 Mbits/sec |
| 上行（板→主机，`-R`） | 30 s | 143 MBytes | 40.0 Mbits/sec |
| 双向（`--bidir`） | 30 s | 下行 49.0 MBytes / 上行 106 MBytes | 下行 13.7 / 上行 29.6 Mbits/sec |

与改动前相比：上行（板→PC）由 7.30 升至 40.0 Mbits/sec，提升约 448%；下行（PC→板）由 20.3 升至 35.2 Mbits/sec，提升约 73%。

仅验证了 aic8800d80 的 SG2002 荔枝派开发板效果，没有验证 aic8800dc 型号。
