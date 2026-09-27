# AIC8800 第二阶段性能优化方案

## 1. 阶段边界

本方案维护 2026-09-27 OCR 审查之后的工作。OCR 审查全文保存在 `ocr-review-20260927.md`，它把此前已经完成的传输形态探索与后续协议、观测和无线能力工作分开。第一阶段的过程、提交和板测证据保留在 `aic8800-optimization-tracker.md`，本文件只把其结论作为第二阶段的输入。

### 1.1 第一阶段输入

第一阶段已完成并提交的工作包括 credit 本地记账、credit 回退粒度调整、事务和 owner 探针、TX 四帧聚合及帧流布局修复、可配置聚合上限、RX 扫描保持实验和 owner park 分段观测。主要可复用结论如下：

| 结论 | 证据 | 对第二阶段的影响 |
| --- | --- | --- |
| credit 本地记账有效 | 周期 1：CMD52/包由约 1 降到约 0.01 | 保持现有记账与失效条件，不重复设计 |
| TX 聚合有效 | 周期 P6：K=1 约 8.42 Mbps，K=4 约 18.7~19.5 Mbps | 保留现有帧流布局，后续只做选值实验 |
| RX defer 不适合作为默认优化 | 周期 P8/P8b：非零窗口未带来收益，3 ms 使固件传输显著变慢 | 默认值改为 0，非零值只作显式实验配置 |
| owner 与 SDIO 总线不可直接视为并行 | 周期 P3/P4/P6 及 OCR 复核 | 不先增加线程或并发提交，先修正协议流量和无线能力证据 |
| 普通 data confirmation 是无消费者的额外流量 | OCR 对照厂商 `rwnx_tx.c:647-671` 与当前 `protocol.rs` | 第二阶段首轮先关闭普通 data confirmation，并重新计数 |

这些结果不构成“已经达到 40 Mbps 或 58 Mbps”的证明。上行在不同会话之间存在显著波动，后续收益必须以实际协商速率、重传、事务计数和同一测试会话中的观测为依据。

### 1.2 第二阶段成功标准

第二阶段首轮的代码目标是恢复普通 Ethernet data 的厂商一致 confirmation 语义，建立可区分 FIFO 项类型的性能观测，并使默认 RX 扫描不被已否定的延迟策略阻塞。可观察验收条件如下：

1. 普通 data descriptor 的 `hostid` 为 0，普通 data 不产生 firmware data confirmation；
2. EAPOL 和需要 firmware 结果的内部控制帧仍沿专用 confirmation 路径；
3. probe 能分别报告普通 data、data confirmation、control confirmation、indication 和 firmware print；
4. 未配置 FDT 属性时 `rx-defer-ms` 为 0，显式非零配置仍可用；
5. 连接、WPA2 四次握手、TCP TX、TCP RX 和双向路径没有新增错误、重传或停滞；
6. 实体板卡结果记录完整，但在没有同会话对照时只报告观察，不宣称精确吞吐收益。

## 2. 第二阶段首轮实现

首轮只修改已经有明确代码证据支持的行为，不把 capability、DMA 所有权和多事务在飞同时加入。这样一份镜像即可回答协议流量是否被清理以及默认 RX 策略是否安全。

### 2.1 Confirmation 与 typed telemetry

`drivers/net/aic8800/src/protocol.rs` 的 `TxConfirmation` 表达普通 data 与 firmware confirmation 的区别。`drivers/net/aic8800/src/tx.rs` 的普通 Ethernet TX 选择 `TxConfirmation::None`，因此 host descriptor 的 `hostid` 为 0；`drivers/net/aic8800/src/device/data_plane.rs` 的内部 EAPOL/控制发送继续选择 `TxConfirmation::Firmware`。普通 data 的 SDIO 写完成已经是 token 完成依据，不能再依赖一个没有消费者的 data confirmation。

`drivers/net/aic8800/src/device/probe.rs` 将接收 FIFO 项按协议角色计数。计数只作为观测，不改变完成、credit、取消、RX 优先级或 IRQ 顺序。特殊帧的 confirmation 仍需后续补唯一 correlation ID、消费者、超时和取消回收；首轮不把这个独立问题与普通 data 处理混合。

### 2.2 RX defer 默认值

`drivers/net/aic8800/src/device/data_plane.rs` 的 `DEFAULT_RX_DEFER` 改为 `Duration::ZERO`。`drivers/net/aic8800/src/rdif/device/endpoints/device.rs` 继续从该默认值构造适配策略，`drivers/ax-driver/src/net/aic8800/fdt.rs` 继续允许 `aic,rx-defer-ms` 在 `0..=10` ms 内显式覆盖。

零窗口意味着 CARD_INT 事实立即进入扫描路径；非零窗口仍保留用于后续同会话实验，但不再作为产品默认。对应单测必须区分“默认立即服务”和“显式窗口保持”，避免测试把实验配置误当默认契约。

## 3. 后续优化顺序

首轮板测完成后，按观测结果推进后续工作。每一步都先保留当前可启动的镜像和代码路径，再增加一个主要变量。

### 3.1 D80 HT40/SGI 能力实验

P0a 实板已完成：COM6 活跃数据窗口中 `data_confirmations=0`，确认普通 Ethernet data 的 `hostid=0` 语义在板上可见。Windows cmd 直接运行 iperf3 后测试端波动减小。

HT40/SGI 已上板两轮（板端启动日志打印 `d80-ht40-sgi`）：正向 25.3/28.8/26.7/29.9 Mbps、反向 18.8/15.6/19.4/19.0/20.6 Mbps、双向 13.6/12.5 Mbps，均高于 P0a 轮同向读数，但两轮不是同一会话，按观察记录。上一轮"`-b 100M` 明显偏慢"没有复现（正向三个用例中它居中，反向四个用例中它与不带 `-b` 持平），该测试端变量可以关闭。板端发送窗口的 `credit` 长期见底（`min=2`、回退 500~1300 次/2 s），说明 TX 仍受固件/空口排空速度限制。

D80 的 `MeConfigProfile`（`drivers/net/aic8800/src/lmac.rs`）按显式偏移安全编码 112 字节 `ME_CONFIG_REQ`，DC 使用保守字节。HT40/SGI 段的取值为 LDPC、20/40 MHz、SGI20/40、MCS32 与单流 MCS0–7 mask、HT highest-rate 150；`device/startup/mod.rs` 只按已验证的芯片身份选 profile 并记录 profile 名。payload 测试固定关键字段与字节；不以 C struct 指针转换或 `repr(C)` 作为线格式编码方式。

速率遥测轮的复测读数（`com6-board-20260928-042935.log`）把这一段的结论定死了：259 条样本全部 `width=20MHz format=ht-mf nss=1 sgi=1`，MCS 以 7 与 6 为主、并有少量 5~1，`txfailed=0`，ack 失败约为成功的 15%。即 **HT 侧能力已生效并工作在 HT20 顶档（PHY 65~72 Mbps），带宽仍是 20 MHz**；20 MHz 上已无 MCS/GI 余量，能动的只剩调制格式。

由此更正一处判读：HT40/SGI 轮曾据厂商 Linux 基线的 UDP 76.5 Mbps 推断该热点支持 40 MHz（依据是它超过 20 MHz 单流 HT 的 72.2 Mbps 上限）。厂商 D80 在 2.4 GHz 同时宣告 VHT 与 HE，20 MHz 的 PHY 上限因此是 86.7 Mbps（VHT MCS9）到 129~143 Mbps（HE MCS11），厂商基线完全可以发生在 20 MHz。**AP 的 BSS 宽度重新成为未知量**，差距的确定部分是调制格式：厂商是 HT+VHT+HE 客户端，本驱动此前是 HT-only。

厂商源码的自然 ABI 对照已固定：标准构建 `CONFIG_RWNX_TL4=n`，`mac_htcapability` 为 32 字节、VHT 为 12 字节、HE 为 56 字节，`me_config_req` 的尾部 scalar 从 offset 100 开始；D80 的 `rwnx_set_ht_capa()` 在 HT40/SGI 路径将单流 MCS32、`rx_highest=135/150` 与 capability bits 一起设置。Rust encoder 只复现这些已核实的字段，不依赖 C ABI cast。

厂商参考是 `lmac_mac.h` 的 `mac_htcapability`、`lmac_msg.h` 的 `me_config_req`、`rwnx_mod_params.c:rwnx_set_ht_capa()` 与 `rwnx_msg_tx.c:rwnx_send_me_config_req()`。HT40 路径设置 MCS32 mask 和 `rx_highest=135`，随后 SGI40 将 highest rate 更新为 150；D80 在厂商启动中为单流。固件和厂商 `lmac_types.h` 的标准构建使用 8-bit `u8_l`，配置结构按自然 ABI 对齐得到现有 112-byte 布局。实际协商 bandwidth/MCS/retry 尚未从 Starry 侧读取，板测结果只报告连接、稳定性与吞吐观察，不将性能变化归因于协商档位。

**本轮（VHT/HE 能力，提交 `1fb1bc91c`）**：`lmac.rs` 新增类型化 `VhtCapabilities` / `HeCapabilities`，D80 profile 更名为 `D80Ht40SgiVhtHe`（启动日志打印 `d80-ht40-sgi-vht-he`），取值照厂商 `rwnx_set_vht_capa()` / `rwnx_set_he_capa()` 在 2.4 GHz、单流下的结果：VHT `vht_capa_info=0x03987111`、单流 MCS 0–9（map `0xfffe`、highest 390）；HE `mac_cap_info[2]=0x02`、`phy_cap_info=[0x06,0xe0,0x2b,0x58,0x0d,0xc0,0xcf,0,0x02,0x30,0]`、单流 MCS 0–11（`mcs_80=0xfffe`）、160/80+80 不支持、PPE 阈值 `[0x38,0x1c,0xc7,0x01,…]`；标量 `vht_supp=1`、`he_supp=1`、`he_ul_on=0`。同一改动把 D80 的 `phy_bw_max` 由 40 MHz 改为厂商对 D80 强制使用的 80 MHz（`PHY_CHNL_BW_80`）；厂商 HT 块的 RX_STBC 与 MAX_AMSDU 位属接收侧能力，本轮不跟。DC 保守 profile 字节不变。

厂商 `me_config_req` 的逐字节对照结论（先排除消息布局错位、A-MPDU 与 `tx_params` 取值、关联请求 IE、信道表 flags 四类假设）与 `phy_bw_max` 的偏差记录见瓶颈文档 §2.16。

本地验证：`cargo fmt --all`、`cargo xtask clippy --package aic8800`（base / `rdif` / `host-test` 三组）、`cargo xtask test --since dev` 通过；新增 VHT/HE 金样本单测（固定每个能力字节、MCS map、PPE 阈值与 HE 结构的对齐填充）并扩展 startup 路径的两芯片断言。镜像 `sg2002_starryos_wifi_sta_vhthe_20260928.img` 基于 P0a 镜像只换内核与 `lcn-sta-defer0.dtb`，构建不执行板卡写入。

上板判据是单一的格式判别：读数出现 `format=vht` 或 `format=he-su` 且 MCS 进入 8–11 → 差距可归因到调制格式；仍是 `format=ht-mf mcs=7` → 该 AP 不与本客户端协商 VHT/HE，方向转向 AP 侧与带宽问题，而不是继续加客户端能力位。只要 `width=20MHz` 保持，任何吞吐变化都不得解释为带宽收益。回滚点为 `sg2002_starryos_wifi_sta_stainfo_20260928.img`（等价 `8a350076a`）与 P0a 镜像。

另有一个板载网络栈的独立项：`net/ax-net` 的 `NEIGHBOR_TTL = 300 s` 到期会重发 ARP 并在 `ETHERNET_MAX_PENDING_PACKETS = 128` 装满时丢包，两轮板测各出现一次、都伴随该用例内的 TX 吞吐下滑（详见瓶颈文档 §2.14）。它不是 AIC 驱动问题，是否处理由上层决定。

### 3.2 TX 聚合选值

现有 `aic,tx-aggregation` 和 `aic,tx-aggregate-bytes` 已能在 FDT 中选择一笔 CMD53 的帧数和字节上限。普通 data confirmation 清理并完成新基线后，再分别针对大数据帧和小 ACK 流测试：

| 负载 | 首选变量 | 候选值 |
| --- | --- | --- |
| 板端 TX 大帧 | 字节上限，帧数作安全上限 | 6/12/24/48 KiB |
| 板端 RX 时产生的小 ACK | 帧数上限，字节上限同步放宽 | 4/8/16/32 |

写成形仍采用机会式冲刷，不为等待更多帧增加固定延迟；credit 仍是 firmware buffer 数量约束。每个候选值至少观察 TX、RX、双向、写事务长尾、RX 事务数量、credit 和是否停滞，选择达到平台峰值且不破坏双向公平性的最小值作为默认。

### 3.3 数据面流水线与 DMA

只有在 confirmation、协商速率和聚合形态稳定后，才根据阶段计时决定是否引入 `staged_tx`、可复用 batch buffer、move-only completion 或 exact-size DMA pool。当前单 owner、单 SDIO transaction in flight、`CardIrqWait` 和 `completion-before-card` 顺序都是安全不变量；任何改变都必须重新说明完成匹配、取消、资源回收和 RX 防饥饿。

复制和 DMA 优化不能以“可能更快”作为立项依据。应先把 owner 推进拆成 RDIF 收割、核心状态机、构帧/聚合、DMA/ADMA 准备和 rearm，再用阶段计时确认 CPU 准备是否占据显著比例。没有该证据时保持现有跨内核边界。

## 4. 验证与板测

代码验证使用仓库任务入口，确保普通 crate、RDIF 和标准库测试组合都能发现真实实现。每轮先完成格式化与静态检查，再构建镜像；探针输出中的计数必须来自 owner 权威状态，不能把历史混合计数继续用于新的吞吐折算。

### 4.1 本地验证

```text
cargo fmt --all
cargo xtask clippy --package aic8800
cargo xtask test --since dev
git diff --check
```

首轮重点检查 `protocol.rs` 的 hostid 布局、`tx.rs` 的 confirmation 选择、聚合/credit/取消/RX 状态机，以及默认和显式 RX defer 单测。修改平台适配时再运行相应的 `ax-driver` 定向检查。

### 4.2 镜像构建

首轮复用已有第一阶段 SG2002 镜像的 rootfs 和 boot 资产，只替换当前工作树编译的内核与 `www/sg2002/wifi-sta/lcn-sta-defer0.dtb`。构建来源和输出由 `sg2002-image-build` 技能维护的 `sg2002-image-build.sh update-kernel` 记录，不把镜像或中间产物放进仓库。

```text
STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 \
  cargo xtask starry build \
  -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml

sg2002-image-build.sh update-kernel <phase1-base.img> \
  --kernel target/riscv64gc-unknown-none-elf/release/starryos.bin \
  --dtb www/sg2002/wifi-sta/lcn-sta-defer0.dtb \
  -o <phase2-output.img>
```

构建完成后检查 FIT 默认配置 `config-sg2002_licheervnano_sd`、kernel load/entry `0x80200000`、DTB 内容和 rootfs 分区完整性。D80 HT40/SGI 实验镜像已输出到本地构建产物目录，文件名为 `sg2002_starryos_wifi_sta_ht40sgi_20260927.img`，SHA-256 为 `9f17b53862d33e82cf1e68c3213b82f622fa07d7fc9a6f85633228cfae554cc7`；它基于 P0a 镜像且未覆盖 P0a（P0a SHA-256 为 `7a3baac1de11214e9bb529985768542722c7cbcd41759a1b21d8ea4dd238feb8`）。本轮不执行 `dd`、`ostool` 或其他板卡写入操作。

### 4.3 板测协议（2026-09-28 起）

**两个敏感操作：重启 PC 热点、重启板卡。** 会话间波动目前归到这两者（2026-09-28 的两遍之间同时发生，未分离到其中某一个）。
测量期间两者都必须受控：要么全程不动，要么在测量前统一执行并记录，禁止把其中一个当作"修好了"的手段而不记录。

**每轮测量前重启 PC 热点并等待约 1 分钟**，使所有轮次都落在"热点刚开"的同一状态；记录本轮开始时间、热点在线时长
与板卡自本次上电起的运行时长。每轮先跑一遍标准三向（60 s 各方向）作为**参考臂**，参考臂不在历史正常区间就丢弃该轮的对比数据。
测试期间 PC 端保持代理关闭。测量时记录 PC 侧热点信道与 station 接口状态（信道、是否有自身流量）：
单射频分时与热点侧状态是当前最大的会话间变量（见瓶颈文档 §2.17）。

每次烧录只在同一次启动内完成连接与握手 smoke test，再跑 Windows cmd 直接执行、无 `-b` 的完整 TCP 板端 TX、
板端 RX 和一次双向；记录 capability profile、`tx_data_confirmations`、`rx_data_frames`、control
confirmation/indication、firmware print、TX/RX transaction、IRQ、scan、credit、重传和停滞，
并把遥测的 `format`/`mcs` 分布与 ack 计数作为该轮的**会话健康记录**归档：出现 legacy 档或 MCS 大面积塌陷时
标注该轮数据不用于归因。

回滚镜像与历史日志只作回退参考：**跨会话的吞吐差异不再被当作收益证据**，只有同一热点会话内、参考臂正常的样本
才用于轮间比较。若新镜像启动或关联失败则回滚到已保留的镜像；不预先安排额外的烧录次数。

## 5. 回滚与交付边界

代码回滚通过保留第一阶段提交和 P0a 镜像完成；FDT 的非零 `aic,rx-defer-ms` 仍可作为实验回退值，但默认值不再恢复为已被板测否定的 1 ms。HT40/SGI 实验镜像默认输出新文件，不覆盖 P0a 镜像。

HT40/SGI 轮在代码验证和镜像自检完成后，交付实验镜像路径及 buildinfo；实体板卡烧录和测试结果另行记录。未取得协商速率、同会话对照或稳定的 typed telemetry 前，不把 40 Mbps、58 Mbps 或固定百分比提升写成验收结论。
