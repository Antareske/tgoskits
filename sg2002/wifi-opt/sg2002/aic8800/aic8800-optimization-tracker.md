# AIC8800 数据面优化跟踪

本文件按「改动 → 测试 → 现象」的周期跟踪 AIC8800 数据面优化的每一步推进。
方案与判据见 `aic8800-async-optimization-plan.md`；每轮的原始数据与单点结论记在该轮的专文里，
本文件只做索引式追踪，不重复专文内容。

## 阶段索引

**第一阶段**是 OCR 审查（2026-09-27）前的已提交探索，覆盖 credit 本地记账、事务探针、TX 聚合、RX 扫描实验和 owner park 观测；其主要成果是 K=4 聚合把上行从约 8.42 Mbps 提升到 18.7~19.5 Mbps，同时确认非零 RX defer 可能让固件退化。第一阶段的历史周期和原始日志保持不变。

**第二阶段**以 `ocr-review-20260927.md` 为边界，从审查提出的普通数据 confirmation 与 typed telemetry 开始。首轮 P0a 的普通数据 hostid、typed FIFO 计数、默认 RX defer=0 已完成代码验证和实体板测；D80 HT40/SGI 能力 profile 已完成实板两轮测试；速率遥测轮（`MM_GET_STA_INFO`）已完成实板复测，读数显示链路稳定运行在 20 MHz / HT-MF / MCS 5–7 / SGI=1；D80 profile 随后按厂商取值补齐 VHT 与 HE 能力并修正 `phy_bw_max`，实板两遍显示 HE 协商生效（`format=he-su`、MCS 9~11），但同一镜像两遍差异极大（发送 26.9 对 2.50 Mbps），且两遍之间同时重启了热点与板卡。

> **口径告诫（2026-09-28 起）**：板测吞吐与 PC 热点状态强相关，同一镜像两次上电可差一个数量级（见「热点状态是跨会话第一变量」）。
> 会话间波动目前归到**两个敏感操作：重启 PC 热点、重启板卡**——2026-09-28 的两遍之间这两者同时发生，因此尚未分离到其中某一个；
> 测试中这两者必须作为受控变量（要么全程不动，要么在测量前统一执行并记录）。因此**跨会话的吞吐对比一律不作为收益证据**；
> 只有同一热点会话内、且当轮读数健康的样本才用于归因。每轮测量按文末「板测协议」执行。
> 第二阶段各轮的提交与镜像对应关系见「分支与提交」的镜像表。后续工作与开放项见文末「待办与下一步」。P0a 镜像保留为回滚镜像。

**阶段二的处置（2026-09-28）**：四轮工作（P0a、HT40/SGI、速率遥测、VHT/HE）全部保留——没有性能衰减的证据，
但也没有性能提升的直接证据（会话间第一变量是 PC 热点状态）。未收敛的收尾项（`-b` 复测、ARP 表项到期、
聚合选值、owner 分段计时、`0x140b` credit 偏移核对）转入待办、不阻塞后续。阶段二**暂时搁置且不予否定**，
当下方向为「通用性下的硬件配置 + 驱动层异步优化」，见执行方案 §3.4/§3.5。

**第三阶段进展（2026-09-28）**：四轮已完成——观测裁决轮（判主机侧完成路径）、聚合上界轮（K=4→32）、发送环对照轮（环 32→64，否定该旋钮）、
credit 等待轮（小写占比 29.3%→7.4%、写中位 23.0→29.3 KB、发送 36.9 Mbps）。主机侧聚合粒度已到本轮实测的上界；
接收侧由卡的交批节奏决定。各轮的数字口径与审查后的更正见瓶颈文档 §1 与 §2.27；
位置与下一步见执行方案 §6.5，**设计保留清单（哪些提交留、哪些被否定）见 §6.8**。

### 第三阶段首轮审查遗留待办（2026-09-28）

审查判定为 `REQUEST CHANGES`，阻断项全在文档侧（已在瓶颈文档 §1/§2.22/§2.23/§2.25/§2.26/§2.27/§2.28 与执行方案 §6.5/§6.8 更正）。
下列为实现与资产侧的建议，尚未处理：

| 项 | 内容 | 触发条件 |
| --- | --- | --- |
| 入库 DTB 的可复核来源 | 被追踪的设计文档只列属性名、不记取值，仓库内无对应 DTS；第二个分支改同一二进制会静默丢一侧 | 上游 reviewer 追问，或再有分支改这个 DTB |
| AKA 板 DTB | `aka-00-sg2002.toml` 启用了 aic8800，但该板 DTB 里一条 `aic,` 属性都没有 ⇒ 同驱动两板默认不同（AKA 仍是 4 帧 / 6144 B） | 在 AKA 上测吞吐或做两板对比前 |
| `aic,rx-defer-ms` 校验 | 仍放行 `0..=10`，其中 3 ms 已实测把链路打垮 | 下次改该旋钮的解析时 |
| credit 等待的常量 | 阈值 8 / 300 µs / 预算 8 是代码常量、未扫值；单次等待实测 703 µs（标称 300），预算打满时约 5.6 ms/笔 | 同会话对照轮一并处理 |
| 探针钩子的位置 | `note_irq_serviced` 是探针专用时间戳，却长在公开的 `rdif-eth` trait 上（跨三 crate + 进程级静态） | 该接口被别的传输层复用前 |
| 环满丢弃分支 | 没有计数器 | 需要观测丢弃时 |
| 默认上界并存 | 1 帧 / 4 帧 / DTB 32 帧三条默认路径并存 | 新增 `AicDevice` 构造路径时 |

**第三阶段首轮 OCR 审查（2026-09-28）**：独立会话的 4 名实例（`principal` ×2、`performance`、空口/接收各一）对第三阶段五个提交做审查，
裁定 `REQUEST CHANGES`（4 阻断 / 8 should fix / 7 suggestion，**全部在文档口径侧，无代码改动要求**）。
主要更正：加权每字节成本的两个口径被混用（"−22%"应为按笔 −12.9% / 按字节 −2.8%）、
接收侧曾用批上限除平均间隔、空口 `mpdu/ampdu` 曾取最后三个样本、`irq split` 的"精确闭合"是恒等式、
credit 等待的收益定级越过"只在同会话对照下宣称"的纪律（已下调为机制已确立、收益待判定）。
更正已落到瓶颈文档 §1/§2.22/§2.23/§2.25/§2.26/§2.27 与执行方案 §6.5/§6.8。
该轮唯一的代码改动是一处注释更正（提交 `a6633f3f6`）：`lmac.rs` 的 `RcStats` 不再把未与厂商核对的布局与比值写成既成语义，
并记明 `avg_mpdus` 是 16.16 定点（实测 520378 ⇒ 7.94 MPDU/A-MPDU，与比值的样本中位 8.0 一致）。
其余为只读审查，无行为改动。

**第三阶段**以 `ocr-review-20260928-bottleneck-round1.md` 为边界。该轮以"驱动瓶颈到底在哪"为问题，
由 7 名 reviewer 独立判定并做三方交叉质证，**否掉了第二阶段的两个方向性前提**：
TX 受固件缓冲池排空限制（其证据链由两个探针窗口拼成）、RX 限在交付节奏（"50% 占空比"分子口径不全，真实占用 58%~69%）。
第三阶段的前提是"每笔事务固定开销 + 事务间空档决定事务率"：固定项中位 445~480 µs 且 2 块以上几乎不随字节缩放，
其中至少一半已实测为主机侧（`owner 20.65% + poll 3.33% + park_sw 12.50% = 36.5%` 墙钟），
而每笔字节数当时被默认的 4 帧 / 6144 B 钉死（credit 并不系统性夹取）。该前提随后被四轮推进改写：见本节上面的「第三阶段进展」与执行方案 §6.5/§6.8。

历史周期中的 M/P 编号和旧分析方案的“阶段 1/2/3/4”是当时局部计划的批次编号；本节所述的两阶段边界以 OCR 审查为准。

| 周期 | 日期 | 改动（要解决的问题 / 对应方案） | 测试 | 现象 | 状态 |
| --- | --- | --- | --- | --- | --- |
| 基线 | 2026-09-17 | —（现状测量，非改动） | STA iperf3 三方向 | 上行 6.61 / 7.30 Mbps，下行 16.0 / 20.3 Mbps，双向板→PC 3.32、PC→板 1.85 Mbps；UDP 与多流用例会卡死板子 | 完成 |
| M | 2026-09-24 | 临时探针：只加计数器，不改数据面行为（为方案 §4 提供实测依据） | STA 镜像 + iperf3 三方向，读串口探针行 | credit 常态 75–119（min 2 / max 132）；CMD52 往返 62–75 µs（总线仅约 5 µs）；回退实测 1.36–1.40 ms（设定 1 ms）；每包 1.21–1.48 ms，其中 650–800 µs 无法归因；txq 积压样本 75% | 完成 |
| 1 | 2026-09-24 | credit 本地记账 + 缩短回退粒度（方案 §5.1 / §5.2） | 单测 + clippy + 板测（探针保留） | CMD52/包 1.0 → 0.01（约 100 倍）；回退均值 1.4 ms → 0.53 ms；上行 TX 包速率 674 → 790 包/秒（+17%）；同口径上行 TCP 6.09 → 9.09 Mbps；无重传；CMD53 边际往返反而 +200 µs（见下）；板端仍会卡死，且本轮发生在上层（见现象 4） | 完成 |
| P2 | 2026-09-24 | 窗口化探针（不改行为）：按窗口统计，并按帧长 / cache 命中 / gap 归因拆分 | 板测（同用例） | 周期 1 的「CMD53 +200 µs」判为帧长构成的假信号；纯上行用例中驱动事务通路空闲（核心队列 90% 为空）；**双向用例队列饱和、gap 由「完成→续发」主导 ≈540 µs/包** → 阶段 2 抓手确认 | 完成 |
| P3 | 2026-09-25 | 供给与总线占用探针（不改行为）：事务按类别记账、gap 按窗口内 RX 事务分桶、写完成时采样两层队列深度，并在执行器侧同步统计调度耗时 | 板测（下行 + 上行 + 双向） | 写方向的成本随长度剧增（512 B 148 µs 对 1536 B 700 µs）；上行的帧早在 RDIF 环里等拉（88%），不是上游缺帧；TX 的 gap 是双峰：无 RX 事务 74–81 µs（72–78%）、有 RX 扫描 2.1–2.4 ms（22–28%）；执行器 60–64% 时间在睡，软件不是瓶颈 | 完成 |
| P5 | 2026-09-25 | 最小聚合（**改动行为**）：owner 一次交 4 帧、核心把 4 个线上帧拼成一笔 CMD53，credit 按包扣、完成按包回 token；同轮修掉完成回环的缓冲滞留 | 板测（上行 + 双向，共 5 个用例） | 写事务层面的聚合生效（一笔 4 包的写往返 1102 µs，对 K=1 的 3 块写 706~780 µs），但四个 TX 方向用例都停住（每次恰好 256 KB），驱动侧空闲且计数平衡、无错误；停住层次未判定（原「停住点在驱动之上」的依据已作废，见 P6） | 完成（停住待定位） |
| P6 | 2026-09-25 | 修复 P5 审查结论（`a8d6fca2e`）：聚合写改为逐帧 4 字节对齐、整笔补 512；内部写不再携带用户帧；在飞的写在停机/取消/批次入队失败时归还 token；推迟完成纳入活性判定 | 单测（6 条红-绿）+ fmt + clippy + `cargo xtask test --since dev`；镜像 `…aggr4fix_20260925.img`；板测上行 10 s / 下行 30 s / 上行 30 s / 双向 | 停住消失：上行 **18.7 / 19.5 Mbps**（K=1 基线 8.42 Mbps，2.3×）；下行 **30.4 Mbps**（P4 27.3）；双向 30 s 正常（板端 TX 12.5 + RX 6.5 Mbps）；首轮双向用例仍停住，但与 K=1 日志同形（见 P6 现象） | 完成 |
| P8 | 2026-09-27 | 接收扫描的有界推迟（`4b4987b27`）：CARD_INT 事实可先攒，仅当「有数据帧待发」且「接收流是单块小帧」时推迟，上限 1 ms；探针增加 `scans`/`deferred`/`tx_ready` 三个计数 | 单测（2 条红-绿）+ fmt + clippy + `cargo xtask test --since dev`；**未上板** | 上板前不可判读；预期把上行 RX 事务条数压下来（见周期 P8 判据）| 待板测 |
| P7 | 2026-09-27 | 聚合的正式形态（`2e017ced6`）：一笔写的帧数与字节上界改为由适配层策略给出，经 `aic,tx-aggregation` / `aic,tx-aggregate-bytes` 配置（默认沿用 P6 取值）；冲刷策略与帧流布局同步进设计文档 | 单测（1 条红-绿）+ fmt + clippy（aic8800 3 项、ax-driver 51 项）+ `cargo xtask test --since dev`；未上板 | 默认取值下行为与 P6 一致；两个上界的作用面不同（见周期 P7 结论） | 完成（选值轮未做） |
| P4 | 2026-09-25 | 写成本探针（不改行为）：写按线上帧长分桶（1/2/3 块）、事务跨几个 owner 轮次、IRQ 到 owner 的唤醒延迟、等待按「通知唤醒 / 到期」分开 | 板测（九个用例，含双向/上行 30 s） | 写成本由长度决定：同一窗口内 512 B 写 127 µs、1536 B 写 759 µs（最小值仅 101 µs），空中队列假设被否定；等待 100% 由通知结束、无到期超时；失败的那次双向用例是上层连接停滞 | 完成 |
| 第二阶段首轮 P0a | 2026-09-27 | 普通 Ethernet data 使用 `hostid=0`，typed FIFO 计数区分 data、data confirmation、control confirmation/indication、print；默认 `rx-defer-ms` 改为 0 | `cargo fmt --all`、`cargo xtask clippy --package aic8800`、`cargo test -p aic8800 --features host-test,rdif`（152 单测 + 1 公共 API 测试）、`cargo xtask test --since dev`（14 个受影响包）均通过；构建镜像并完成 FIT/boot/rootfs 自检；COM6 与 Windows cmd 原始日志已归档 | Windows cmd 直连测试后，完整 TCP 轮次包括板端接收 16.4/20.7 Mbps、双向板端 TX/RX 约 9.69/6.14 Mbps、反向板端 TX 16.0 Mbps；活跃窗口 `data_confirmations=0`。`-b 100M` 反向完整轮次板端 TX 为 6.71 Mbps，需单独复测；中止轮次不作为完整吞吐结论 | 代码、镜像与首轮板测完成 |
| 第二阶段 HT40/SGI | 2026-09-28 | 将 `ME_CONFIG_REQ` 改为类型化 profile；D80 只启用 HT40+SGI 和对应单流 HT MCS mask/rate，VHT/HE 仍关；DC 保守 profile 不变 | vendor C 声明与构造路径只读对照（标准 `CONFIG_RWNX_TL4=n` ABI）；Rust 单测 + 三组 AIC clippy + `cargo xtask test --since dev` 通过；独立镜像 FIT/boot/rootfs 自检通过；两轮板测共 11 个用例，原始日志已归档 | 板端确认运行 `d80-ht40-sgi`：正向 25.3/28.8/26.7/29.9 Mbps（P0a 轮 16.4/20.7），反向 18.8/15.6/19.4/19.0/20.6（P0a 轮 16.0），双向 13.6/12.5（P0a 轮 9.69/6.14）；均为跨会话观察。`data_confirmations=0` 保持；板端发送窗口 credit 长期见底（`min=2`、回退 500~1300 次/2 s）。另定位 ARP 表项 300 s 到期事件与一次偶发断链，详见下文 | 实板完成 |
| 第二阶段速率遥测 | 2026-09-28 | `MM_GET_STA_INFO`（`0x0075`，4 字节 compat 载荷）每 1 s 读一次协商速率/RSSI/ack 统计；读数只作观测，超时或坏确认只停读数不判链路失败 | vendor 结构、位域与发送路径只读对照；`cargo fmt --all`、三组 clippy、`cargo xtask test --since dev`（162 单测 + 1 公开 API 测试）通过；新增 7 条单测；镜像 `sg2002_starryos_wifi_sta_stainfo_20260928.img`，SHA-256 `8954110d907340ab323369d9c004fc6d0ccea6a3d523d1b1995210457040f23b`，FIT 内核与 `starryos.bin` 逐字节一致、Load `0x80200000`、默认配置 `config-sg2002_licheervnano_sd`、rootfs `/bin/sh` 与 `/starryos.uimg` 校验通过 | 复测（`com6-board-20260928-042935.log`）恢复正常吞吐：接收（PC→板）30.5 Mbps、发送（板→PC）23.5 / 22.9 Mbps、双向 19.1（板发）/ 10.4（板收）Mbps。259 条读数全部 `width=20MHz format=ht-mf nss=1 sgi=1`，MCS 以 7（95 次）与 6（93 次）为主、另有 5（56）、4（10）、3（3）、2/1（各 1）；`retries=2` 恒定、`txfailed=0`、`rssi` −12~−16 dBm；ack 计数到末尾为 362135 成功 / 66444 失败（约 15%，会话早期一度接近 1:1，随后改善），停止发包后计数冻结 | 实板完成 |
| 参照·同床对照 | 2026-09-28 | 观察（无代码改动，除新增 `[wifi-stack] is_5g_support` 一行，提交 `aae244c88`） | 厂商 Linux 与 dev 主线内核在同一天、同一热点（SSID `aasta`）各跑一轮，日志归档为日志目录下的 `linux-wifi/` 与 `dev主线wifi/` | ① 厂商最好轮 发送 56.5 / 接收 50.4 / 双向 48.9-12.1 Mbps（姿态不佳的轮次只有 5.35~22.1，说明姿态对厂商同样致命）；② 结论：**差距与频段无关**（同一热点、Windows 热点单频段、本板 2.4-only 能连 → 2.4 GHz；且 56.5 Mbps 在 5 GHz VHT80 下只占 13% 不合常理），**差距在同一频段/带宽下均匀约 2 倍**；③ **双向接收塌陷两栈同形**（厂商 24% 对 我们 27% 的跌幅）→ 移出待归因项；④ 厂商日志含 `Mode: SD High Speed (50MHz)` → 当时判为"SDIO 速率成为首要候选"；**该判读已更正**：该行是 u-boot 对 `SD_HS` 的枚举标签且属于 microSD（`cv-sd@4310000`），相邻 `Bus Speed: 25000000` 才是实测速率；**厂商 Wi-Fi SDIO 的实际时钟在记录中无读回**（厂商 DTS 上限 25 MHz，但 BSP 对 D80 请求 150 MHz 且绕过内核钳位）→ 该项降级为"待一次读回定论"，见瓶颈文档 §2.20 | 完成（遗留一次厂商侧读回） |
| 当下方向·协商读数与姿态变量 | 2026-09-28 | 观察（无代码改动，除告警条件收窄）：分析含新证据行的实板日志 | `com6-board-20260928-180304.log`（正确姿态）+ 两份扰动日志；告警条件收窄的改动经 171 单测与三组 clippy 通过，提交 `a86512cf8` | ① **40 MHz 开放项关闭**：`[wifi-assoc] width=0`、`ap ht40=0` → 该 AP 的 BSS 就是 20 MHz；`ap he=1` 佐证厂商基线高吞吐来自 HE。② **A-MSDU 生效**：`our_amsdu=Some(true)`、`ap ht_amsdu=1`、`amsdu=` 计数出现 147/258 的窗口 → 下行确实在用 A-MSDU。③ **固件自述** `features=0x01e877d7`（vht/he 开、amsdu 特性位关、amsdu_max=2、ant_div=0）——厂商按 `min(modparam, amsdu_max)` 仍会宣告 A-MSDU，故硬编码声明与该固件下的厂商行为一致。④ **会话变量再定位**：扰动轮 MCS 塌到 3~7 而 `rssi` 反而更高（−23 对 −26）→ 发射侧变差，与天线姿态/放置相符；正确姿态轮 RX 31.3/31.2、TX 27.2、双向 21.3/8.39 | 完成（下一轮板测按新协议确认姿态后再比数字） |
| 当下方向·证据型 info | 2026-09-28 | 让"配置"与"协商"两类不明确处都能自证：启动读一次 `MM_VERSION_REQ` 打印固件版本与特性字（含 A-MSDU 支持与最大尺寸、VHT/HE、ant_div）；关联指示解出固件认定的频点/带宽与 assoc req/rsp 元素，打印对端 HT/VHT/HE、HT Operation 带宽位、A-MSDU 位、A-MPDU 指数与 DS 信道，并标出本驱动请求里是否带 A-MSDU 位；A-MSDU 交付计数进 `[wifi-probe-rx]` 的 `amsdu=` | `cargo fmt --all`、`cargo xtask clippy --package aic8800` 三组、`cargo xtask test --since dev` 通过（169 单测，含新增的固件特性解码、协商解码与"字段缺失仍可连接"三条）；提交 `139347056`；**第二轮 OCR 审查（round 2）抓到三个真缺陷并在 `a366b59ee` 修复**：新启动阶段 `ReadVersion` 的条件互斥会让启动必失败（Critical）、`sm_connect_ind` 偏移整体偏低 2、固件特性位序整体偏低 2；镜像 `sg2002_starryos_wifi_sta_negotiation_20260928.img`（**修复后重建，SHA-256 `a97548ae7ba54a71a04f2d07609448aeedfaaadf087cfde32fcafdf743347448`**；此前同名镜像含启动缺陷，已删除替换，勿使用旧哈希），FIT 内核 crc32 `9864b321`、自检通过 | 待板测：上板即可从 `[wifi-fw]` 与 `[wifi-assoc]` 两行读出"AP 是否支持 40 MHz/A-MSDU""固件自报的 A-MSDU 能力"等证据 | 待板测 |
| 对照镜像·dev 主线内核 | 2026-09-28 | 为对比主线（dev）性能，用主线内核替换本工作树镜像的内核，其余资产（rootfs、DTB、payload）全部复用 | 在临时工作树 `wt-dev`（detached `d3536651c`）编译 dev 内核（`AIC8800_FIRMWARE_DIR` 用离线缓存），再以 `update-kernel --kernel <dev>/…/starryos.bin` 装入基镜像；FIT 内核 crc32 `e11120e6`（本工作树内核为 `8d6c8eca`，两者确实不同）、尺寸 16410976 B、自检通过。**注意**：`update-kernel` 不做编译，只传 `--commit` 会静默复用工作区上一次的内核（本次首建即因此出错，已重做） | 镜像 `sg2002_starryos_wifi_sta_dev_20260928.img`，SHA-256 `e4f0dd57969394fc9dff15ddd0946d78352873fbeb3f897b63b5288a48297551`；其 `.json` 的 `commit` 字段记的是本工作树 HEAD（脚本不记录 `--commit` 的 rev），实际的 dev 提交是 `d3536651c`，以本行为准 | 已完成（待用户测） |
| 当下方向·RX 交付放大 | 2026-09-28 | D80 profile 声明 HT `MAX_AMSDU`（A-MSDU 接收上限 7935 字节），让 AP 可用 A-MSDU 发送、固件每笔交付载荷更大；接收路径本就有 A-MSDU 解包分支 | `cargo fmt --all`、`cargo xtask clippy --package aic8800` 三组、`cargo xtask test --since dev` 通过；金样本与 startup 断言更新（D80 capability info `0x0063` → `0x0863`，并断言该位与"保守 profile 不含该位"）；镜像 `sg2002_starryos_wifi_sta_amsdu_20260928.img`，SHA-256 `cebf412fde7444b0b91d2a54f8d40ce758b2f13fb5121d3dee7e698c9a92ad39`，自检：kernel 尺寸 16435552 B、crc32 `7864980e` 相符、Load `0x80200000`、默认配置与 `/bin/sh`、`/starryos.uimg` 校验通过，`.json` 记录提交 `ac62a558a` | 待板测：判据是 `[wifi-probe-rx]` 的读大小分布（`<=512/<=2k/<=8k/>8k`）与 `data_reads`/字节——若 AP 接受 A-MSDU，应看到大块读变多、每字节读事务减少 | 待板测 |
| 当下方向·写管线续接 | 2026-09-28 | 写完成后立即续排下一笔写（缓存 credit 用尽则先排流控读），续接限定每轮协议推进至多一次；完成事件、token 归还、单笔在飞与完成先于 card 事实的顺序不变 | `cargo fmt --all`、`cargo xtask clippy --package aic8800` 三组、`cargo xtask test --since dev` 均通过；新增 3 条行为单测（续接一次后让位给接收扫描、缓存用尽时先读流控寄存器、取消时丢弃已排队的写），并把既有延窗用例更新到新时序；两条续接用例经变异验证（删掉续接即失败） | 提交 `2772a4c0e` 后做了一轮 OCR 审查（5 个 reviewer 实例），结论 REQUEST CHANGES：取消路径会留下已武装的写 → `CompletionMismatch` → 设备 `Failed`（4/5 独立指出），另有"新测试无红-绿判别力"等应修项；修复提交 `dfdc282cc`：放弃管线处窄清 `io.next`、发出决策归一为一个 helper、修正探针语义并拆分续接计数、补齐取消路径用例（变异验证：去掉修复即失败）。镜像 `sg2002_starryos_wifi_sta_txchain_20260928.img`，SHA-256 `4cf1c4a5757eea7c53c1fd04b244476eacac40363473ab72e598eabbdcc7e765`，自检：FIT 内 kernel 尺寸 16435552 B 与 `starryos.bin` 一致且 crc32 `badffd84` 相符、`boot.sd` Load `0x80200000`、默认配置 `config-sg2002_licheervnano_sd`、`/bin/sh` 与 `/starryos.uimg` 校验通过，构建时 `.json` 记录的提交为 `dfdc282cc`<br>**实板完成，未见性能提升**（`com6-board-20260928-162606.log`）：续接确实在生效——大流量窗口里 `chain write` 占写次数约 37%、`chain flow` 5%、`yield`（上界让位）42%、`idle` 17%；但周期由**单笔写自身**主导（1.66 ms 周期里写占 1.06 ms，64%），空档只剩 207 µs（1056/1203 笔）与 3.4 ms（147 笔）。更关键的是最忙窗口 `credit` 均值仅 7.1、`backoff` 225 次/秒、`min=2`（贴着保留位）——**发送方向受固件缓冲池/空口排空限制，不是提交调度**，故续接无处发力。该轮吞吐：接收（PC→板）32.8/32.1 Mbps（历次最好）、发送（板→PC）17.3/19.3、双向 17.0/8.63；读数全为 `he-su`、MCS 6~11，但低档占比高于上一轮（`mcs=7` 76 次、`6` 46 次），会话状态不可与上一轮直接比较<br>建议：TX 侧停止在此投入（限制项在固件/空口），异步线转向 **RX 侧**（RX 事务占总线时间约 81%，且 RX 是当前较好方向） | 实板完成（无收益） |
| 第二阶段 VHT/HE 能力 | 2026-09-28 | 与厂商 `me_config_req` 构造路径逐字节对照后扩展 D80 profile：新增 VHT 与 HE 能力块（单流 MCS 0–9 / 0–11、40 MHz-in-2G、LDPC、PPE 阈值等，取值照厂商 `rwnx_set_vht_capa()` / `rwnx_set_he_capa()` 在 2.4 GHz 单流下的结果），把 `phy_bw_max` 由 40 MHz 改为厂商对 D80 强制使用的 80 MHz；DC 保守 profile 不变 | 对照项与结论见下文「第二阶段 VHT/HE 能力」小节；`cargo fmt --all`、`cargo xtask clippy --package aic8800` 三组、`cargo xtask test --since dev`（aic8800 通过）均通过；新增 VHT/HE 金样本单测并扩展 startup 路径断言；镜像 `sg2002_starryos_wifi_sta_vhthe_20260928.img`，SHA-256 `c8dc9ccd2a6ebc8f1939df9017a8e431baecdcb6f92480a78ff11f4b5ac0a7e9`，自检：FIT 内 kernel 尺寸 16435552 B 与 `starryos.bin` 一致且 crc32 `f3f7c289` 相符、Load `0x80200000`、默认配置 `config-sg2002_licheervnano_sd`、`/bin/sh` 与 `/starryos.uimg` 校验通过 | 代码已提交 `1fb1bc91c`；镜像已构建并自检<br>**实板完成（两遍）**：HE 协商生效——第二遍读数全部 `format=he-su`、MCS 以 9~11 为主（11:60、10:38、9:29），吞吐 接收 30.6 / 发送 26.9 Mbps；第一遍（同一镜像）只有 15.7 / 2.50 Mbps，读数为 `he-su` 低 MCS 与 `non-ht mcs=3` 混跳、ack 失败约 31%。两遍之间**同时**重启了热点与板卡，因此成因未归因；详见下文「热点状态是跨会话第一变量」 | 实板完成（性能变量待分离） |
| 参照·SDIO 时钟出处追查 | 2026-09-28 | 只读调查（无代码改动）：查清本板 SD/SDIO 拓扑、时钟策略及其决策链，并追查"超过 25 MHz 不稳定"这句话的出处与强度 | `clock.rs:29-53`、板级 DTB（`cv-sd@4310000` / `wifi-sd@4320000`）、厂商板级 DTS 与 BSP/内核 SDHCI 路径、`git log -S "becomes unreliable"`、记录内全部 `SD High Speed` 出现处 | ① 两路控制器独立（microSD 走 SDIO0、AIC8800 走 SDIO1），改 Wi-Fi 那路不影响卡槽，但速率策略在共享的 `clock.rs` 里；② 25 MHz 封顶的**唯一**依据就是 `clock.rs` 的两行注释（提交 `0fc626fa4`，与该次 50→25 MHz 改动同一处 diff），无配套文档/测试/测量，且与早期笔记"补上 PHY delay 后 50 MHz 跑通"的记录相左；③ 要提高需先补 UHS-I 信令表达与 tuning（本项目均无），只改 DTB 无效；④ 更正：`Mode: SD High Speed (50MHz)` 是 u-boot 对 microSD 的枚举标签（实测行是 `Bus Speed: 25000000`）；"厂商被平台钳到 50 MHz"无代码支撑（厂商 D80 请求 150 MHz 且绕过内核钳位）；⑤ 记录中**无 Wi-Fi SDIO 时钟读回** → "两侧同为 25 MHz"只是 DTS 推断，该项降级为待一次读回 | 待一次读回（厂商侧 `debugfs` 的 mmc `ios`）；在此之前不投入 UHS-I/tuning 实现 |
| 第三阶段·完成路径分段与聚合上界对照 | 2026-09-28 | ① 把设备中断的**入口与出口**都交给驱动，使一行往返分成「设备+总线 / 中断处理本身 / 唤醒并取走完成」三段；② 把写跨度的 credit 最高桶细分为 `16-33 / 34-65 / 66+`，用于回答"能否一笔 32 帧"；③ DTB 变体 `lcn-sta-aggr32.dtb` 把 `aic,tx-aggregation` 设为 32、`aic,tx-aggregate-bytes` 设为 49152（32×1536，远小于环形上限 32×2048） | `cargo fmt --all`、`cargo xtask clippy --package aic8800`（三组）与 `--package ax-net`（九组）、`cargo xtask test --since dev` 全部通过；提交 `e01089b60`；两份镜像同源内核、只差 DTB，FIT/boot/rootfs 自检均通过 | 待板测（**必须在同一热点会话内成对完成**，两轮之间只重启板卡并记录姿态与 MCS 分布）：基线臂 `…_split_20260928.img`（K=4）与实验臂 `…_aggr32_20260928.img`（K=32）。判读：① 实验臂的 `size blk` 分布与 `bytes/笔` 是否真的形成 32 帧（约 48 KB）；② `write credit 34-65 / 66+` 桶的笔数决定 K=32 在多少比例的窗口里可用；③ 两臂的 `irq split` 里 `isr` 与 `post` 两段各占多少（基线臂的 post 为 396 µs/笔）；④ 吞吐只作观察，机制读数（`bytes/笔`、跨度对 B 的斜率、`accounted`）才是判据<br>实板完成（A1/B1/A2 三臂，同热点会话，§2.23）： **K=32 生效且按模型奏效**——字节/笔 5.6 KB → **18.7 KB**、写笔数减半、每字节成本 189 → **133~144 ns/B**（1.42×，与 `F + r·B` 一致）；<br>发送 **22.6 → 30.9 → 25.1 Mbps**、双向板发 21.4 → 26.0 → 20.2、双向板收 7.60 → **13.2** → 9.66；<br>B 轮 MCS 9~11 占比 **80%**（A1 60% / A2 65%）、低档样本更少，与"B 轮波动但无突然退化"的观察一致；ack 失败率三臂相近（29/26/26%）。<br>**新发现**：K=32 下写长随 credit 增长（10.1 / 22.8 / 26.7 KB）且**从未到 48 KB** ⇒ 限制已从策略上界转为**固件 credit**；<br>低 credit 桶（3–6）占比从 12% 升到 23.4%、每字节 261 ns/B（是 34–65 桶的两倍）。<br>`irq split` 三分段：**`isr` 中位仅 2 µs**（中断处理本身可忽略），`post` 317~367 µs 绝对值不变但占比从 35% 降到 13.6% | 完成 |
| 第三阶段·接收侧三条验证 | 2026-09-28 | 为零成本验证三个问题做读数：① 卡片报的块数分布与上限（`rx count empty/blocks/bytemode/other`、`blocks max`、`blocks` 分桶）；② refill 节奏（从"读到空"到"再看到数据"的间隔，>0.1 s 记为 idle）；③ mask 语义（`irq rearm n= / card_pending= / completion_pending=`，重武装时卡中断是否仍 asserted） | `cargo fmt --all`、`cargo xtask clippy --package aic8800`（三组）与 `--package ax-net`（九组）、`cargo xtask test --since dev` 通过；提交 `4c07118b5`；镜像基于聚合臂、DTB 用入库板级文件（自带 32 帧上界），FIT/boot/rootfs 自检通过 | 待板测：① `blocks max` 与 33-64/65+ 桶是否有样本——若封顶在约 43 块则 21~22 KiB 就是卡侧上限，"读得更大"没有空间；② `refill avg` 若聚在某一固定值附近即为卡自定时聚合，若很小则限制在我们的轮询步调；③ `card_pending` 占 `rearm` 的比例<br>实板完成（a1/b1/a2，同热点会话，§2.25）：① **块数最大 63、`65+` 桶零样本** ⇒ 实测批上限 = 63 块 = 31.5 KiB（64~127 之间无样本）；② refill avg ≈ 7.5~8.7 ms，同口径折算（平均批 42~45 块 ÷ 8 ms）≈ **21 Mbps**，低于实测接收吞吐（31~34 Mbps）⇒ 卡的交批节奏足以供上主机，主机侧无余量；③ `card_pending` 仅 **1.8%~2.4%** ⇒ 通知基本不丢，43% 空档不是"等下一次边沿" | 完成 |
| 第三阶段·发送环深度对照 | 2026-09-28 | 厂商在聚合那一刻从 64/8192 深的队列 pull，我们 push 进 32 槽环、取完即 break，写长的真实上界是"那一刻环里有几帧"（瓶颈文档 §2.24）。实验臂把 `aic,queue-size` 由 32 提到 64（环 64×2048=128 KB，`aic,tx-aggregate-bytes=49152` 仍合法），内核与基线臂完全相同、只差 DTB | 与基线臂同源内核（`4c07118b5`），FIT/boot/rootfs 自检通过；零代码改动（纯 DTB） | 待板测（**与基线臂在同一热点会话内成对**，两轮之间只重启板卡）：基线臂 `…_rxprobe_20260928.img`。判读：① `bytes/笔` 是否从 18.7 KB 升向 32 帧的 48 KB；② `write credit 34-65 / 66+` 桶的 `bytes/笔` 是否也上去（若上去 ⇒ 之前确实是"环里没帧"，若不动 ⇒ 供给本身不足）；③ `accounted` 与 `tx 忙` 是否随之上升；④ 吞吐只作观察<br>实板完成：**写确实变大但吞吐没动**——字节/笔 19.7 → 23.0 KB、credit 34–65 桶 21.7 → 33.3 KB（ns/B 137 → 126），但 credit 3–6 小写占比从 18% 翻到 **29%**（263 ns/B），**按笔加权的 ns/B 基本不变（171.8 / 174.9 / 167.3）**、发送 34.2 / 34.8 / 33.5 Mbps ⇒ 环深不是约束（判据是吞吐不变；按字节加权整段口径下环 64 反而略低，见瓶颈文档 §2.25） | 完成（否定） |
| 第三阶段·credit 低时先等（对齐厂商） | **已实现并实板验证**（`3cb53bb17`） | 厂商在 credit 见底时**原地等**（`FLOW_CTRL_RETRY_COUNT=50`，预算约 126 ms），我们只登记 200 µs 退避后用小写把 credit 花掉。改成"credit 低于一档就等"直接冲着那 29% 的 263 ns/B 小写去（按笔加权口径下它们占了加权成本的近一半，按字节口径只占 5.0%）；需一轮对照确认不引入停顿 | 已实板：小写占比 29.3% → 7.4%、写中位 23.0 → 29.3 KB、发送 36.9 Mbps、慢尾收缩；每字节成本同口径为按笔 174.9 → 152.4、按字节 139.3 → 135.4（后者的幅度在同会话自然波动内）⇒ **机制成立、收益幅度待同会话对照判定** | 见 §2.27 | 完成 |
| 第三阶段·观测裁决轮 | 2026-09-28 | 零行为变更的观测轮（M1 写跨度按提交时 credit 分桶、M3 按设备中断把往返切成"设备+总线"与"主机取走"两段、M6 按时间口径的派生读数；探针口径修正与遥测口径修正），见执行方案 §6.3 | `cargo fmt --all`、`cargo xtask clippy --package aic8800`（三组）与 `--package ax-net`（九组）、`cargo xtask test --since dev` 全部通过；提交 `0fce56495`；镜像 `sg2002_starryos_wifi_sta_observe_20260928.img`，SHA-256 `2802b6b1f0e499706d2f319879524a197ca7e5f303d084d9e01005f0bb6e930d`，FIT 内核 crc32 `c4e67b95`、Load `0x80200000`、默认配置 `config-sg2002_licheervnano_sd`、rootfs `/bin/sh` 与 `/starryos.uimg` 校验通过 | 待板测：① `[wifi-probe-time]` 的 `write credit 2/3-6/7-15/16+` 分桶——跨度随 credit 单调上升则判设备反压、基本平坦则判主机侧；②同行的 `irq split` 给出每类事务"中断前/中断后"两段；③`accounted` 与 `supply` 三档的按时间占比；④`[wifi-sta-info]` 的 `ackok_d`/`ackfail_d` 逐秒增量<br>实板完成（`com6-board-20260928-194254.log`）——吞吐与上一版同级（接收 32.2、发送 28.3、双向 22.7/9.97 Mbps），符合零行为变更；三组读数互证：<br>① **M1**：保留位桶（credit 0–2）**0 笔**；跨度随 credit 下降而**变短**（788 对 1036 µs），因为写里帧更少（3189 对 5453 B），每字节反而更贵（247 对 190 ns/B）⇒ **设备反压不成立**；最忙窗口三桶拟合 `F≈604 µs、r≈91 ns/B`；<br>② **M3**：`irq split` 的两段之和等于该笔跨度是计时口径本身（非独立验证），写往返 **pre 724 µs / post 396 µs（35%）**，RX 大读 pre 2150 / post 245（10%）⇒ 那 35% 按构造全是设备报完成之后的主机软件时间；<br>③ **M6**：饱和窗口写事务占窗口 73~77%、accounted 83~85%、`supply none` 约 7%；纯 TX 整段的 39% 有 22.9 s 落在 16 个未饱和窗口 ⇒ 饱和时供帧不是限制项；RX 测试 rx 事务占窗口 48~55%、每笔 22.3 KiB、105~108 ns/B。<br>**归属判为主机侧完成路径**（ISR → 唤醒 → 恢复 → 取走完成），去掉 post 段后同一窗口的发送上限约 43 Mbps。遗留观测缺口：credit 最高桶（16+）过宽（要判"能否一笔 32 帧"需知有多少笔 credit ≥34）、post 段内部未再分段、IRQ 戳可能被同窗 CARD_INT 覆盖 | 完成（归属已判定） |
| 参照·瓶颈归属 OCR（首轮，独立会话） | 2026-09-28 | 只读审查（无代码改动）：以"驱动瓶颈到底在哪"为问题，7 个 reviewer 独立判定 + 三阵营交叉质证；完整报告归档 `ocr-review-20260928-bottleneck-round1.md` | `dev...HEAD` 全部驱动改动 + 全部板测日志逐窗重算 + 厂商参考实现只读对照；未运行任何构建/测试/静态检查 | ① **收敛结论**：瓶颈形状是「每笔固定开销（中位 445~480 µs，与承载字节基本无关）+ 事务间空档」决定事务率，**不是每字节速率**——两个方向的边际成本相同（写 9.11~10.43、读 9.88 MB/s，即贴 11.72 MB/s 总线）；② **三条既有结论被否**：C1（TX 限在固件/空口）证据链由两个窗口拼成且核心分解是恒等式，C2（RX 限在交付节奏）分子口径不全（真实占用 58~69% 而非 50%），"纯上行受上游供帧限制"是按计数关闭、按时间是窗口 21~29%；③ **固定项里至少一半已实测是主机侧**：`owner 20.65% + poll 3.33% + park_sw 12.50% = 36.5% 墙钟 / 360 µs 每笔写`；`handoff`（中断→执行器恢复，239~346 µs/笔）按构造 100% 是内核软件时间，机制候选为同权重 Fair 线程的条件抢占被拒；④ **`packets=4` 是真天花板**（两个入库 DTB 都没设该旋钮；credit 并不系统性夹取：`credit max` 中位 131），上调估计 1.87×；⑤ 时钟翻倍上界 1.15~1.56×（<2×），不作为主攻方向；空口税 24~28% 真实但不可叠加 | 待两条**裁决测量**：M1（每笔写跨度按提交时 credit 分桶）+ M3（SDIO 提交/完成边界时间戳）；另有零代价项 M-K1（打开已注册未启用的 `sched:sched_switch`）与 M6（按时间重算已打印字段） |

| 参照·接口规格外查与本地考证 | 2026-09-28 | 就"卡侧块计数语义 / CARD_INT mask 语义 / 卡是否自定时聚合"三问向外检索，并把两份外部结论逐条拿到本地厂商树上复核 | 厂商源码只读对照（`aicwf_sdio.c`/`aicwf_sdio.h`/`aicsdio.c`/`soph_base.dtsi`）与我们的 `registers.rs`；未运行构建或烧录 | ① **证实**：流控单位为 1536 字节缓冲槽（`BUFFER_SIZE=1536`，门控按字节 `len < buf×1536`）；D80 **不做** `&0x7F`（掩码只对 8801/DC/DW）；块计数 `&0x7F`、`120` 为 byte mode；bit7 = dev→host soft IRQ 需读改写清 bit0；寄存器偏移 0x01/0x03/0x04/0x05 与我们逐项一致；② **纠错**：外部结论称"wifi 节点声明 `sd-uhs-sdr25/ddr50/sdr104`"——实际 `sd-uhs-*` 只在 microSD 节点，且 `sd-uhs-ddr50` 在设备树里根本不存在；③ 双方均**未找到**：流控寄存器位数/最大值、卡侧 RX 批的触发规则、私有 0x01/0x04 位语义 | 完成（登记层可排除） |
| 第三阶段·每秒分布与波动归因 | 2026-09-28 | 只读分析（无代码改动）：把三臂的每秒 iperf3 序列与同刻 `[wifi-sta-info]` 对齐，判断"波动"是主机侧还是空口侧 | 现有日志逐秒重算 | ① **三臂都能冲到 40+ Mbps**（A1 有 8 秒 ≥40、B1 4 秒、A2 1 秒），每秒中位几乎相同（35.5 / 35.5 / 34.4）⇒ 均值差异在噪声内，**峰值不是 B 独有**；② 均值低于中位说明损失在**慢秒**（最低 16.8 Mbps）而不是峰值不够；③ 慢秒与链路质量同向：A1 慢秒 MCS 8.1、B1 慢秒 rssi −23（对快秒 −14）⇒ 形态属空口/姿态，不属主机路径。原判读里同时引用的"ack 失败 33% 对 24%"自 §2.26 起**不再作为链路质量证据**（该计数不是每帧统计），此处只保留 MCS 与 rssi 两项 | 完成 |
| 参照·ack 统计口径更正 | 2026-09-28 | 只读核对（无代码改动）：核实 `[wifi-sta-info]` 里 `ackok`/`ackfail` 的单位 | 厂商 `lmac_msg.h` 的结构定义 + 三臂逐秒计数与 MPDU 速率对比 | ① 字段真实名是 `ack_fail_stat`/`ack_succ_stat`，厂商驱动**全树从未使用**；② `(ackok_d+ackfail_d)/MPDU = 0.21~0.32` ⇒ 跳动频率只有 MPDU 速率的 1/4~1/5，**不是每帧统计**；③ 原"首次尝试失败率 24%~28%"等表述全部降级为"口径未定"（瓶颈文档 §2.26）；④ 顺带发现同一确认里 **`chan_time`/`chan_busy_time`/`chan_tx_busy_time` 三个信道时间字段从未解析**，其中 `busy/time` 可直接给出信道占用比例 | 完成 |
| 第三阶段·空口统计与 credit 等待 | 2026-09-28 | ① 每 2 s 读一次 `ME_RC_STATS_REQ`（`0x140e`），解析 `me_rc_stats_cfm` 前缀：`mpdu`/`ampdu` 计数与固件自算的 `avg_mpdus`，并打印 `mpdu/ampdu` 比值——这是唯一能说明"空口每聚合装几个 MPDU"的读数；② credit 落在保留位之上、批次阈值（8）之下时，写不再用小突发把 credit 花掉，而是等一档再问一次（每次 300 µs、至多 8 次，超预算则照发；空池仍走原有 200 µs 退避）；③ 修正 `0x140b` 的命名（它是 credits update，traffic ind 是 `0x140d`） | `cargo fmt --all`、`cargo xtask clippy --package aic8800`（三组）与 `--package ax-net`（九组）、`cargo xtask test --since dev` 全部通过（含新增的 rc 统计布局 golden 测试与改写后的 credit 保留测试）；提交 `3cb53bb17`；镜像基于环 64 臂、DTB 带 `rng-seed`，FIT/boot/rootfs 自检通过 | 待板测：① `[wifi-rc]` 行是否出现、`mpdu/ampdu` 是多少（若远小于 32，说明空口聚合深度才是我们与厂商的差距所在）；② credit 3–7 占比是否下降、加权 ns/B 是否下降；③ 发送吞吐与每秒波动是否改善<br>实板完成（`com6-board-20260929-011955.log`，§2.27）： **credit 等待把小写（credit 3–6）的笔数占比从 29.3% 压到 7.4%**、每字节成本同口径为按笔 174.9 → **152.4（−12.9%）**、按字节 139.3 → **135.4（−2.8%）**（后者落在同会话自然波动内）、写中位 23.0 → **29.3 KB**；发送 **36.9 Mbps**（每秒中位 37.8、均值 38.0、最低 23、低于 30 的仅 6 秒），对照上一会话三臂 34.8 / 34.2 / 33.5 且慢尾更深。**空口读数首次取得**：`[wifi-rc]` 的 `mpdu/ampdu` 在有流量的 63 个样本里中位 **8.0**、上四分位 **15.0**、最大 **40.0**，与我们每笔交出的约 20 帧同量级 ⇒ **不能据此认为"空口用不掉更大的 K"**，该方向回到开放。`ampdu_len` 是固件采样区间的累计量（非 2 s 窗口量）、`avg_mpdus` 为 16.16 定点数 | 完成 |

| 第四阶段·设备层时钟与全归因探针 | 2026-09-28 | 把时钟借给驱动（`AicRdifOptions::probe_clock`，平台填 `axklib::time::monotonic_nanos`），据此把一笔事务拆成 `dispatch/dma/program/bus` 四条腿（`bus` 由**结束该事务**的中断界定），把 owner 侧工作拆成 `release/rx_copy/pull/teardown/form/bytes/rx_parse/report` 八段（`form` 在 `prepare_next_transmit` 内部打点，覆盖链式续写；`bytes` 在唯一做该拷贝的函数里打点）；`advance` 行按所取完成的类别归档、无完成的步归 `other`；修掉上一轮归因探针接错路径（整场零样本）与运行时逐中断打戳（卡中断会把宿主唤醒记进 `bus`）两处；两臂的每笔字节拷贝计数改为对称，并注明 `tx_scratch` 实际未被复用 | `cargo fmt --all`；`cargo xtask clippy --package aic8800`（三组）、`--package ax-net`（九组）、`--package rdif-eth`、`--package ax-driver`（51 项）全过；`cargo test -p aic8800 --features rdif,host-test`（178 + 1）；`cargo xtask test --since dev`；板级内核 `licheerv-nano-sg2002-wifi.toml` 构建通过。新增一条判别性单测（advance 费用归到它取走的完成所属类别；对"丢掉无完成步"的旧行为验证为失败） | **一轮 OCR 审查（6 名实例：principal ×2、quality ×2、performance、reliability）：REQUEST CHANGES**，5 项 blocker / 6 项应修 / 3 项建议；blocker 全部按审查结论修完，其中最重要的是"链式续写的成形未被计时"（本轮原本量不到它要量的那条路径）与"完成中断无身份" | **实板完成**（`com6 …065738`，§2.29）：仪器闭合成立（`pre` = 四条腿之和，逐窗 ±9 µs / 0.3%）；`clock=on`、`agg=32x49152`、framed 路径。纯 TX 10 窗（80.769–98.786 s）：375 笔/窗、21063 B/笔、周期 5310 µs、往返 2458（46%）、`bus` 1869、`dma` 140、`program` 53、`post` 389、`P`(pull+form+bytes) 379；双向 10 窗（104.789–122.796 s）：298 笔/窗、24923 B/笔、周期 6459、往返 2803（43%）、`bus` 2218、`dma` 154、`post` 372、`P` 465。**两条新结论**：① `bus` 拟合为边际 11.2 MB/s、截距≈0（原始的 96%；接收 99.5%），第三阶段"每笔固定开销 445~604 µs"是旧往返口径产物；② 空档占周期 54%/57%，其中 credit 退避 1017/418 µs、其余 1354/2506 µs；机制是完成路径空手结束（`progress.rs:401-406` 对 `WaitForInterrupt` 立即返回），同调用内排在后面的环→核心 pull（`submit_one_tx`）不执行，而 `rdif_at_write` 非空 62.6%/53.9%、`chain idle` 49%/53%、`chain write` 0%。**与并行设计文档对照**：其基础数字全部复现（`P` 379/465、`dma` 140/154、`bus` 1869/2218、`post` 389/372、环非空 62.6%/53.9%），分歧在分母（服务周期 2843 µs 只占实测周期 54%，故 13.3%/18.3% 换成吞吐口径是 7.1%/9.8%）、未列 credit 退避、未用 `chain` 计数器 |

| 第四阶段·完成前交帧（链式续写） | 2026-09-29 | 上一轮认定「完成路径空手结束、环里的帧没人取」，本轮让那一笔能自己续上：owner 的步内顺序由「init → active → control → pull → tick」改为「init → control → **pull** → active → tick」，环里已有的帧在**在飞请求被推进之前**交给核心；这次交付若换来核心的 `WaitForInterrupt`、而此刻仍有在飞请求，则不结束本轮（`handover_keeps_the_step`），继续把该请求推进到完成。判读修正：核心的续写只在**取走完成的那一刻**找帧（`consume_transmit_data` → `continue_transmit_pipeline` → `prepare_next_transmit`），所以 §8.7 原先设想的「返回等待前做一次 pull」排不出续写——把 pull 提到完成之前，完成时核心才有一帧可续，续写被排进 `io.next`，下一次 advance 的**顶部**就会发出它（早于 `drive_ready` 的全部内容，含接收扫描）。**第二个目标（OCR 发现后采纳）**：同一处重排把控制请求块也移到了在飞请求之前，于是控制事务进行期间有一笔写在飞时，取消立即对该写生效（核心 `AbortSdio` → 中止 → 载荷按完成上报并归还 token，续接已排出的写丢弃）。这是核心本来的意图（`cancellation_releases_the_write_in_flight`），旧步序让 `AbortSdio` 在稳态下不可达、并让「写卡住时取消服务不到」，故保留该次序、按新语义改设计文档与注释，不回退。 | `cargo fmt --all`；`cargo xtask clippy --package aic8800`（base / `rdif` / `host-test` 三组，全过）；`cargo test -p aic8800 --features rdif,host-test`（180 + 1）、`cargo xtask test --since dev`（全过）；两条新增单测经变异验证（谓词改恒真、以及关掉 `continue_transmit_pipeline` 各自失败）；板级内核 `licheerv-nano-sg2002-wifi.toml` 构建通过、`strings starryos.bin \| grep -c aasta` = 1；设计文档 `docs/design/unified-sdio-aic8800.md` 的续接条款补入交付时序、取消条款按新语义重写。镜像 `sg2002_starryos_wifi_sta_prechain_20260929.img`（基座 `…_q64_20260928.img`、DTB `lcn-sta-defer0.dtb`，与上一轮臂**只差内核**），FIT 内 kernel 哈希与 `starryos.bin` 逐字节一致、Load `0x80200000`、默认配置 `config-sg2002_licheervnano_sd`、`/bin/sh` 与 `/starryos.uimg` 校验通过，SHA-256 见「镜像与提交对应表」。**一轮 OCR 审查（2026-09-29，6 实例：principal ×2、quality ×2、performance、reliability）：REQUEST CHANGES**，1 阻断 / 3 应修 / 8 建议；阻断项即上面的控制块次序（已按采纳的处置落地），应修项（主体改动无测试守卫、判据口径被本轮重置、设计文档未更新）与建议里的探针口径、`aic,queue-size` 上界也一并落地 | 待板测。**判据（口径已按审查重写）**：本轮**同时动了被测量与观测器**——`supply ring/core`、`rdif_at_write`、`pull avg`、`advance other` 的样本总体与 `gap` 各档的计时起点都被重置，这些量**只在同一窗口内**读、不与上一轮做差；可跨轮比较的是 `chain *`、`tx_writes`、`size blk`/`bytes=`、`period_avg/max`（须与写长度同读）、`scans/deferred/tx_ready`、`credit backoff`。验收读数：① `pull in_flight n=/frames=` 显著非零（旧步序下该打点结构上不可能看到在飞请求，故必然为 0 ⇒ 新次序确实在跑；它是次序的足迹，不是效果的证据）；② `chain` 七项之和 = `tx_writes`（闭合式，被取消的完成不记账）；③ 收益判据落在 `chain write/flow` 上升与`chain idle` 占比下降（上一轮 49%/53%）；若 `pull in_flight frames` 与 `pull frames` 同量级而 `chain write` 仍 ≈0、`chain idle` 不动，则判为「交付了但完成没续接」。不退化项：`[wifi-probe-rx]` 的 `reads`/`bytes`/`rx delivered`、`control n`、往返三项（`bus`/`dma`/`post`）与吞吐。**已知上界**：核心的「每轮驱动至多续一次」（`io.chain_used`，只在 `drive_ready` 里清）使续写只能落在每隔一笔上，故可及收益是「约一半的写提前于接收扫描发出」。详见执行方案 §9 | **实板完成（同一热点会话，10 个 iperf3 用例，板端作 server）**：**机制完全兑现、收益未兑现**。① **机制**（上行相位四次运行均值，全部为窗口内读数）：`chain write` 占完成笔数 **0% → 16~24%**、`chain idle` **51% → 13~24%**、`chain yield` 19% → 34~39%（即「每轮至多续一次」的让位上界，与预测一致）、`supply ring` **46~52% → 0%**、`supply core` 29~33% → 74~84%；新计数器 `pull in_flight frames / pull frames` ≈ **73%**（帧确实在在飞期间被搬走）；`chain` 七项之和 = `tx_writes` 逐窗精确闭合。② **收益**：下行 **28.5/28.6 vs 28.6/27.9 Mbps（持平）**，RX 事务数每字节与每字节成本一致（122 vs 123 ns/B）⇒ 接收侧不退化；上行 **30.1（28.4~32.9）vs 30.2 Mbps（持平）**，周期 5641 vs 5323 µs（+6%，落在同会话逐轮 ±9% 的散布内）；双向板发 25.0/26.6 vs 29.5 Mbps，但该相位的接收形状不同（A-MSDU 942/窗 → ≈0）、接收量少 14%、其中一次运行发生接收塌陷（365 Kbps）⇒ **不可判**。③ **空档分解（本轮新知识）**：上行相位每笔空档 ≈ 3145 µs，其中「哪里都没有帧」（`supply none`，占 17% 的笔 × 8370 µs）≈ 1423 µs/笔，**占空档 45%**；空档相对上一轮的 +304 µs/笔**全部**发生在这个桶（+351 µs/笔）⇒ 是会话供给变了，不是改动所致。含 RX 事务的空档只占 32%，且**纯上行相位里扫描本就很少挡在写前面**（上一轮该相位的 `ring` 档空档均值仅 651 µs，是最快的一档）⇒ 次序改动在纯上行相位无处发力，与实测持平一致。**判读更正**：§3.3 的「环里的帧没人取 ⇒ 空档 2.9/3.7 ms」出自**双向**窗口，适用范围应限定在双向相位。④ **无异常**：无错误/超时/abort；`chain` 的未命中只落在 `idle`/`yield`（`busy`/`backoff`/`not_ready` 全窗为 0），`deferred=0`；链路健康不差于对照（MCS 11 为主、rssi −13~−25 dBm、ack 失败率 23~28% 与对照同档）。⑤ **下一步**：次序已不是纯上行的杠杆（剩余空档最大单项是「没有帧可发」）；双向相位需**同会话 A/B** 才能判定；**round-2 证据与归因审查（5 实例，含厂商对照）：REQUEST CHANGES**，2 阻断 / 4 应修 / 6 建议。两处口径更正：① 周期账（锚在 owner 调用入口）与空档账（锚在即时读钟）是两个估计量、差 300~580 µs/笔，本行此前并列的「总量 3145 µs 与分档合计 2758 µs」不可相加，「+6%」与「增量全部发生在 none 桶」两条**撤回**；② 「哪里都没有帧占空档 45%」**降为推断**（`supply` 是完成时刻的瞬时采样；本轮 `supply core` 占 80.8~85.3% 而该桶空档仍 1.1~1.8 ms）。**并否掉「总线时钟是差距来源」这一候选**（厂商 D80 的 150 MHz 请求是 `#if 0` 死代码：fdrv `aicwf_sdio.c` 3184…3218、BSP `aicsdio.c` 1835…1868 包住了两侧 v3 的时钟块；两边同为 25 MHz 量级、差 1.067×，折整窗约 +2.2%）。收敛结论：全部 SDIO 事务在飞占窗口 45~50%、**无事务在飞 46.9~51.9%**、执行器 park 72~78%（owner 22~23%）、credit 自选等待 15~16%（后者此前后从未出现在任何空档归因里）；正确措辞为「搬运方式不再是可动项，可动项在两次搬运之间」。下一步：零代码的 `lcn-sta-nowait.dtb` 对照（判那 15~16% 值不值）＋ 一条新读数（`chan_*` 解析 / 栈侧供帧间隔 / park 唤醒来源 / 帧年龄，五票无多数）。详见 `aic8800-async-optimization-plan.md` §9.6（已按审查改正）与 `.ocr/sessions/2026-09-29-sg2002-wifi-opt/rounds/round-2/`。 |
| 第四阶段·归因探针深化（park 唤醒来源 / 信道占用 / 栈侧供帧） | 2026-09-29 | round-2 归因审查给出的三个首选读数一次做齐，全部是仪器（不改行为）：① **执行器 park 的唤醒来源**——`QueueNotification` 增带 `WakeReason`（`Irq`/`Transmit`/`Receive`/`Recycle`/`Control`/`Work`），`schedule_irq` 用 `Irq`，三处协议侧发布点分别用 `Recycle`/`Receive`/`Transmit`，控制请求起止用 `Control`，其余归 `Work`；park 前先丢弃陈旧原因、返回后按来源归档（`None` = 到达等待时事件已置位、这次 park 从未睡下）。`[netprobe]` 新增 `park_end irq/tx/rx/recycle/ctrl/other/none/timer`，把执行器 72~78% 的停驻从"睡着了"拆成"在等谁"。② **卡自己的信道时间**——`StationInfo` 解析 `channel_time`/`channel_busy_time`/`channel_tx_busy_time`（`word(12)/word(16)/word(28)`，与厂商 `lmac_msg.h:1287-1297` 逐字段对齐、总长 32 字节与 `STA_INFO_CONFIRMATION_LEN` 一致；厂商主机侧从不读这三个字段），按上一采样做差后随 `[wifi-sta-info]` 打印 `busy=Npermille txbusy=Npermille`，同一秒的信道窗口做分母。用卡自己的读数回答"空口是不是天花板"，不再靠推断。③ **栈侧供帧**——`poll_inner` 入口采样 `tx_ready` 深度（0/1/2-3/4+）与相邻两次提交的间隔，`[netprobe]` 的 `supply` 行补 `gap n/avg/max`；"环里没有帧"（上一轮判为推断的 `supply none`，占空档 45%）由此变成观测。另修两处既有口径：`other` 档改为打印真正的 OTHER 类并把原列改名 `rx_ctrl`；自算的 `period` 均值改按 `period_samples` 计数（此前分母用了写笔数） | `cargo fmt --all`；`cargo xtask clippy --package ax-net --package aic8800`（12 项全过）；`cargo test -p ax-net --lib`（129）、`cargo test -p aic8800 --features rdif,host-test`（180 + 1）全过；板级内核 `licheerv-nano-sg2002-wifi.toml` 构建通过 | 纯仪器轮，无行为改动，不单独送审查；判据在上板前定下（见"状态"） | **实板完成**（`com6 2026-09-30_001748`，同一热点会话内 4 个 iperf3 用例：下行 20 s / 下行 30 s / 上行 30 s / 双向 30 s）。① **仪器成立**：`park_end` 八项之和 = `waits`，**76 个窗口逐窗闭合**；`timer` 只在有 deadline 等待的窗口出现。② **停驻去向（第一条新读数）**：`none` 在所有相位都占 **≈50%**——到达等待时事件已置位，**一半的 park 从未睡下**；其次是 `tx` 25.7%（下行）/12.7%（上行）、`irq` 22.4%/14.0%、`recycle` 1.5~6.8%、`rx` ≈0.2%、`other` 与 `ctrl` 全零；`timer` 下行 0.05%、**上行 18.1%**、双向 14.7%。`wait_kind deadline` 比 `timer` 多出 5~12%，差值是「等到截止时刻才被通知」的那部分，两者不可互换。③ **credit 等待被独立证实**：上行由截止时间结束的 park 合计 **16.1% 墙钟**（4831 ms/30 s、均 680 µs/次），与上一轮三条读数的 15~16% 吻合；次数（7103）与 `credit backoff`（7642）同量级 ⇒ 执行器侧对同一等待的独立记账。④ **栈侧供给（第三条）**：上行 82% `core` / 18% `none` / **0% `ring`**（与上一轮 `ring 46~52% → 0` 一致），下行 85% `none`（只有 ACK）；上行 `supply 4+` 占 10.4% ⇒ 栈是突发式交付而非持续积压。⑤ **信道占用（第二条）读数失效**：77 个采样**全部 `busy=0 txbusy=0`**，含上行 30 Mbps 期间。厂商树里这三个字段（`lmac_msg.h:1292/1293/1296`，都在 `mm_get_sta_info_cfm` 内）**主机侧从不消费**；厂商真正使用的信道时间是另一条消息 `MM_CHANNEL_SURVEY_IND` 的 `chan_time_ms`/`chan_time_busy_ms`（`rwnx_msg_rx.c:292-318`，进 nl80211 survey）。故本轮不能判「空口是不是天花板」；下一轮或改读 survey 指示，或加打印原始值以区分「字段恒零」与「窗口不前进」。**时间账（上行 30 s，全部窗口内读数）**：事务在飞 **38%**（写总线腿 36%、收 1%、控制 0.9%）；写往返 2515 µs/笔 = 总线腿 1951（78%）+ 主机 564；总线效率写 89 ns/B、收 87~96 ns/B ≈ 物理底速。非在飞 62% 中：`supply none`（哪里都没有帧）**20%**、`supply core`（有帧却在等）**23%**——后者**包含**上面 16.1% 的 credit 等待（同一段墙钟，不可相加），其余约 7% 是完成→中断→交接→再发的固有延迟。**吞吐**：下行 27.6 / 30.5、上行 32.0、双向板发 29.2 / 板收 6.31 Mbps；逐秒中位 28.3 / 30.3 / 32.6，无错误、无超时、`deferred=0`。**结论**：搬运方式确已到底（96% 物理底速、`ring` 空档归零），差距在**占空比**（我们 38%）；非在飞时间里「没有帧」20% 属上层节奏，可动的只剩 credit 自选等待 16% 与其外的 7%。六条判据四条给出实读，第③条因读数失效未判。 |
| 第四阶段·前车成形（在飞窗口内排好下一笔） | 2026-09-29 | 上一轮把可动项定在「两次搬运之间」的固有延迟（完成→中断→交接→再发约 7%，加上每笔约 0.5 ms 的成形与 DMA 准备），本轮把其中**成形**一段搬进在飞窗口。① **核心新增 staging 槽**：`DataPlaneState` 增加 `staged_tx`，`active_tx` 仍是「总线槽」——两者的区别是 `active_tx` 已被请求路径读到、`staged_tx` 还没有。`prepare_next_transmit_inner` 在 `active_tx` 空闲时先把 `staged_tx` 提升进总线槽、否则才成形；`form_transmit(limit)` / `extend_write(&mut ActiveTx, limit)` 改为操作局部写，聚合逻辑仍只有一份（顺带删掉 `extend_active_write` 里那条「槽在本次遍历中消失」的防御分支——按 `&mut ActiveTx` 形参它构造上不可达）。`advance_once` 在 `io.pending` 是数据写、状态 `Ready` 时，先做一次有界 `stage_next_transmit()` 再返回 `WaitForInterrupt`，于是 owner 在飞期间的每次交帧都会顺带排好下一笔；核心仍只返回 `WaitForInterrupt`，**不产生第二笔提交**。② **允许提前成形的窗口被收紧到「已经交给总线的写」**：仍在等流控读数的写会按读数继续从 `data.tx` 生长（`consume_transmit_flow` → `extend_active_write`），提前成形的后继会把帧先取走、使它在总线上小于刚读到的 credit 所能支持的规模；因此只有 `IoPurpose::TransmitData` 在飞时才 staging。成形上限再扣掉在飞写将要花掉的包数（`aggregate_limit().saturating_sub(in_flight)`），同一份 credit 不被两笔写重复认领。③ **token 归属**：`take_active_write_tokens()` 同时回收两个槽，取消（`finish_cancel`）、停机（`drive_shutdown`）、失败（`fail`）三条路径各归还一次；`bulk_sender()` 把 staged 写也算作「本方在发载荷」（`rx_defer` 默认 0，当前无行为影响，只是口径更正）。④ **旋钮与探针**：新增 `aic,tx-prepare-ahead`（0/1，默认 1；`AicRdifOptions::tx_prepare_ahead` → `OwnerPolicy::tx_prepare_ahead` → `AicDevice::set_tx_prepare_ahead`），供同会话 A/B；probe 的 `chain` 行新增 `staged ready/missed`（每笔完成时后继是否已经成形）。⑤ **DMA 活动前缀公共契约就位（下一步 owner 双槽的前提）**：`memory/dma-api/src/owned.rs` 增 `CpuDmaBuffer::capacity()` 与 `prepare_prefix(len)`，`DmaError` 增 `InvalidActiveLength`，`PreparedDma`/`InFlightDma`/`CompletedDma`/`QuarantinedDma` 全链路携带活动长度（`len()` 返回活动前缀、`capacity()` 返回整个 backing，cache 同步与 `segment()` 只覆盖 `0..active`，`into_cpu_buffer()` 恢复完整 backing，提交拒绝原样归还）；`prepare_for_device()` 语义不变。消费方 `sdmmc-protocol`/`sdhci-host`/`dwmmc-host` 的 `DmaError` 穷尽匹配补 `InvalidActiveLength` 分支（映射为 `InvalidArgument`）。⑥ **正式文档**：`docs/design/unified-sdio-aic8800.md` 增「前车成形」条款（含「只有已交给总线的写才允许有后继提前成形」与 credit 不重复认领两条理由），并把 token 回收范围补成两个槽 | `cargo fmt --all`（干净）；`cargo xtask clippy --package aic8800`（base / `rdif` / `host-test` 三组全过）、`--package sdmmc-protocol --package sdhci-host --package sdmmc-host --package ax-net`（15 项全过）、`--package ax-driver`（51 项全过）；`cargo xtask test --since dev`（全过）；`cargo test -p aic8800 --features host-test`（161 + 1，含四条新增）。四条新增单测：`a_write_on_the_bus_leaves_its_successor_already_formed`（在飞期间交帧 ⇒ `staged_tx` 非空、`data.tx` 清空、返回仍是 `WaitForInterrupt`，完成把它提升并提交）、`a_write_waiting_on_credit_keeps_the_frames_its_reading_will_add`（流控读在飞时不得 staging，读数回来后写仍长到两帧）、`prepare_ahead_can_be_turned_off_for_the_serial_arrangement`（关掉旋钮回到旧排列）、`shutdown_returns_the_packets_of_the_write_staged_behind_the_bus`（两槽各归还一次）。`dma-api` 的五条前缀测试经变异验证（同步整块 backing / `segment()` 用 capacity 各自失败），但**在本树无法执行**：该 crate 的集成测试目标有既有的 `__SpinOps_acquire/release` 链接缺口，且它不在 `scripts/test/std_crates.csv` 中，故 `cargo xtask test` 不会选中它（见「已知缺陷」） | 已实现、**已板测**（同会话两臂；吞吐判不出差异，机制足迹明确） | **实板完成**（ahead1 四次运行 / ahead0 一次，同一内核同一基座，只差 DTB 一个属性）。① **机制成立且闭合**：`staged ready` 占完成笔数 85–90%（b1 恒 0），`staged ready+missed = tx_writes` 与 `chain` 七项之和 = `tx_writes` 逐窗精确闭合（79/54/98 窗零违例）。② **空档被填了一部分**：`chain idle`（完成时排不出后继）A 10.0/10.0% → B 17.6%；写周期 A 4483–4718 µs 区间。③ **总线占用与吞吐都不动**：总线腿占比 A 35.9/36.0% 对 B 36.1%；上行 iperf A {33.4, 36.9, 37.2, 36.4} 对 B {38.0}，但**两臂判不出次序**——B 仅一个样本，双向 TX 里 A 反快 2.4%（29.50 对 28.8），合池 A 33.20(n=7) 对 B 33.40(n=2) 差 0.6%，逐秒 Mann-Whitney p=0.081。④ **写长度差约 9%，但归因未定**：单向上行 like-for-like A 少 8.6%（合池 11.5%，双向窗 17.5%）；然而 pre-knob 镜像（attrib3）的每笔字节与 A 臂相符、且 b1 自己两半窗口之间就差 2884 B（大于两臂差约 2000 B）⇒ 不能判为旋钮效应。**我曾据此说'A 的分布更紧'，方向是反的**（CV：A 8.9% 对 B 8.5%），那是按绝对四分位比较的尺度错觉。⑤ **代码追踪指出一处实现缺陷（未修）**：`stage_next_transmit` 的 `limit = aggregate_limit().saturating_sub(in_flight).max(1)` 把减法放在策略上限**之外**，credit 充足时把成形上限从 32 帧压到 `32 − s_prev`。判别性证据是该缺口只出现在'照缓存 credit 直接发出、不再走流控读'的带宽带里并随 credit 增大而扩大（3-6 档 A/B 相符，34-65 档 −8.5/−11.5%），但**要证否必须补逐笔 `s`/`s_prev` 插桩**，现有日志做不到。⑥ **下一步**：不用重编，把现成的 `attrib3` 镜像当第三臂，在同一会话里与 ahead0/ahead1 用相同用例序列交错 2–3 次，可一次分清'会话 vs 我的重构公共路径'（两者都含该重构，B 不是它的对照）。 |
| 第四阶段·无等待对照（`aic,tx-credit-wait-us=0`） | 2026-09-29 | 只改 DTB 一个属性：以 `ahead1` 为基准把 `aic,tx-credit-wait-us` 设为 0（`lcn-sta-ahead1-nowait.dtb`，与 `ahead1.dtb` 只差该属性），内核/基座/其余属性全同，构成严格单变量对照。驱动侧改动为零：`consume_transmit_flow` 里 `if !wait.is_zero() && thin && waits < BUDGET` 这条薄池自等待分支被跳过（`thin` = credits 3..=7） | `cargo fmt`（干净）；镜像 `sg2002_starryos_wifi_sta_ahead1-nowait_20260929.img`（`update-kernel` 于基座 `q64`，内核与 ahead1/ahead0 同一份 `57ac05ff…`；从镜像 FAT 抽出 DTB 与源文件逐字节比对相同；脚本自检 Load/默认配置/`/bin/sh`/`/starryos.uimg` 四项全过） | 已板测；**机制足迹明确、总线层面不动** | **实板完成**（`nowait 063542`，与同日的 ahead1 四次运行对照）。① **旋钮确实生效，且足迹很大**：`write credit` 的 3-6 档份额 4.8/4.9/6.4/5.4% → **20.2%**（逐窗 64/68 窗 ≥10%，而 ahead1 四次合计仅 6/203；该位移是 boot 间散布的约 11 倍）；每笔字节 20.6–21.8 KB → **18.6 KB**（10+ 块占比 89% → 75.6%）；写周期 4483–4718 µs → **3914/3984 µs**（−12%，与 ahead1 四次零重叠）；`chain idle` 8.0–8.7% → **6.3/7.1%**；发射相位的 deadline 等待占比 11.0–13.7% → **8.2/10.5%**（按每次写的停驻时间算 −24%）。② **但它买到的东西不在总线上**：总线腿占比 35.8% 对 ahead1 的 35.9/36.0/36.1%；上行吞吐 38.0/35.3 对 36.4/34.6 —— 都不动。③ **代码归因（已核对到 file:line）**：该旋钮只作用于薄池分支（`data_plane.rs` 的 `credits 3..=7`）；**保留量分支（`credits <= 2`，`IO_RETRY` 200 µs）与遥测/邮箱/启动/SDIO 寄存器重试等其余截止时间来源都不受它影响**，前者占 credit 等待约 78%。④ **结论**：那 16.1% 并非空转——等待在等池子回填以便下一笔聚合得更大；去掉它只是把每笔写变小，占空比与吞吐均不变。⑤ **交叉影响**：我此前判断「`chain idle` 只受 prepare-ahead 影响、nowait 不会动它」**是错的**，实测降了 1.2–2.3 点（boot 内散布仅 0.3–0.8）。 |

### 本轮登记的口径更正（跨轮适用，先登记再算数）

1. **「事务在飞 38%」是另一个口径**。此前各处引用的 38% 只算**总线腿**（写 36% + 收 1% + 控制 0.9%）；
   probe 的 `accounted=` 含整个往返（dispatch + dma + program + bus），同期约 50–61%。两者差约 20 个百分点，
   **不可混比**；引用「在飞」时必须写明是哪一种。
2. **RSSI 不能作为「会话/射频」的证据**。`[wifi-sta-info]` 的 rssi 是**下行**方向收到的信号：
   2026-09-29 22:43 那次的 `-15 dBm` 好于 22:47 那次的 `-24 dBm`，却是慢的那一次（28.3/28.9 对 36.4/34.6）。
   用它预测上行走廊不成立。
3. **`064020` 与 `064400` 是两次不同开机，不是同会话重复**，且前者带一段深衰落、固件池也更薄
   （平均 credit 8.5 对 12.3–14.1）。因此不能把两者的 24% 差当作重复性度量，
   也不能据此说「噪声地板超过一切效应」——NOWAIT 的 credit 3-6 档位移是它的约 11 倍。
4. **`supply core/none` 在 prepare-ahead 开启的臂里口径已被改动**：帧已经进了 `staged_tx`，
   而 `supply` 只看 `data.tx`，于是 A 臂的 `none` 虚高（24–26% 对 b1 的 15.9%）。**该量不可跨臂比较。**
   下一轮应先修探针（把 `staged_tx` 计入供给），否则同样的误判会再来一次。
5. **逐秒 iperf3 行不可用**：`log::info!` 会穿插、覆盖串口输出，逐秒行常与探针文本粘在同一行。
   只有整段汇总行（区间自 `0.00` 起）可判读；每臂末尾多出的一条 `Server listening (test #N)` 是「起了没跑」，
   不是丢失的结果行。


## 分支与提交

- **工作分支：`sg2002/wifi-opt`**。AIC8800 的全部工作（含调试与探针）都在此分支上迭代；
  需要开 PR 时另建去掉探针调用的整理分支，不在本分支直接开。
- 第一阶段已提交至 `751555277`。第二阶段 P0a 与 D80 HT40/SGI 的改动在构建镜像时还留在工作树中，随后于 2026-09-27 提交为 `fd80b2b69`（P0a）与 `752f308e0`（HT40/SGI）；速率遥测（`8a350076a`）与 VHT/HE 能力（`1fb1bc91c`）提交在镜像构建之前。OCR 前后的范围以本文件顶部「阶段索引」为准。
- 第三阶段首轮 OCR 审查的代码侧改动只有一处注释（提交 `a6633f3f6`），内容与结论见「阶段索引」的审查段与遗留待办表；其余为只读审查。

### 镜像与提交对应表

镜像由 `sg2002-image-build` 的 `update-kernel` 生成，构建脚本把当时 `git rev-parse --short HEAD` 写进同名 `.json`。下表按镜像列出该字段与镜像内容的对应提交，便于按镜像回看代码。

| 镜像 | 构建时 HEAD | 内容对应的提交 | DTB | 状态 |
| --- | --- | --- | --- | --- |
| `sg2002_starryos_wifi_sta_phase2_p0a_20260927.img` | `751555277` | `751555277` + 工作树的 P0a 改动，即 `fd80b2b69` | `lcn-sta-defer0.dtb` | 已板测（P0a 轮、HT40/SGI 轮均以其为对照） |
| `sg2002_starryos_wifi_sta_ht40sgi_20260927.img` | `751555277` | `fd80b2b69` + `752f308e0` | `lcn-sta-defer0.dtb` | 已板测（HT40/SGI 两轮） |
| `sg2002_starryos_wifi_sta_stainfo_20260928.img` | `8a350076a` | `8a350076a` | `lcn-sta-defer0.dtb` | 已板测（遥测复测轮） |
| `sg2002_starryos_wifi_sta_vhthe_20260928.img` | `1fb1bc91c` | `1fb1bc91c` | `lcn-sta-defer0.dtb` | 已板测（HE 生效；两遍暴露热点变量） |
| `sg2002_starryos_wifi_sta_txchain_20260928.img` | `dfdc282cc` | `dfdc282cc` | `lcn-sta-defer0.dtb` | 已板测（续接命中 37%，未见提升） |
| `sg2002_starryos_wifi_sta_amsdu_20260928.img` | `ac62a558a` | `ac62a558a` | `lcn-sta-defer0.dtb` | 未单独板测（其内容被后续镜像覆盖） |
| `sg2002_starryos_wifi_sta_negotiation_20260928.img` | `139347056` | `a366b59ee` | `lcn-sta-defer0.dtb` | 已板测（协商证据与姿态变量轮）；SHA-256 `a97548ae`（修复后重建） |
| `sg2002_starryos_wifi_sta_observe_20260928.img` | `0fce56495` | `0fce56495` | `lcn-sta-defer0.dtb` | 已板测（第三阶段观测裁决轮，§2.22）；SHA-256 `2802b6b1f0e499706d2f319879524a197ca7e5f303d084d9e01005f0bb6e930d` |
| `sg2002_starryos_wifi_sta_split_20260928.img` | `e01089b60` | `e01089b60` | `lcn-sta-defer0.dtb` | 已板测（成对对照的**基线臂** A1/A2：K=4）；SHA-256 `6986652997d491ee476835e8324980364be9a00dfb8abeaacfc16bcad0703c35` |
| `sg2002_starryos_wifi_sta_aggr32_20260928.img` | `e01089b60` | `e01089b60` | `lcn-sta-aggr32.dtb` | 已板测（成对对照的**实验臂** B1：`aic,tx-aggregation=32` / `aic,tx-aggregate-bytes=49152`）；SHA-256 `a97703986d3b21c102ca00e877cdf4c9f9ce07775ef7523edb6b45892179ff3b` |
| `sg2002_starryos_wifi_sta_q64_20260928.img` | `4c07118b5` | `4c07118b5` | `www/sg2002/wifi-sta/lcn-sta-q64.dtb`（STA 血统 + `aic,queue-size=64`，环 128 KB） | 已构建自检，待板测（**成对对照的实验臂**：加深发送环）；SHA-256 `2784c0698c477304b56c1fc092aa13b3ae5edc963fd298e69a552f6552e134d8`（**再次重建**：2026-09-29 误加 `--overwrite` 就地覆盖成新内核，随后按同一方式（`4c07118b5` 内核 + `lcn-sta-q64.dtb`）重新组装；FIT 时间戳不同故哈希与上一版不一致，勿用旧哈希 `fc1b748d…` / `e593bf32…`） |
| `sg2002_starryos_wifi_sta_airstat_20260928.img` | `3cb53bb17` | `3cb53bb17` | `www/sg2002/wifi-sta/lcn-sta-q64.dtb`（含 `rng-seed`） | 已板测（空口统计 + credit 等待，§2.27）；SHA-256 `c4f357f894b445b8bbf810ed4a3a58c73e1993c083cb1053605be2511403a774` |
| `sg2002_starryos_wifi_sta_rxprobe_20260928.img` | `4c07118b5` | `4c07118b5` | `lcn-sta-defer0.dtb`（STA 血统，自带 32 帧上界） | 已板测（**接收侧三问的基线臂** a1/a2，§2.25）；SHA-256 `f8b1e67fd95c6f08fb48b60784f20256e78a0c9054a7d1e73aa5fb2de9e0e849`（**重建**：首版用入库板级 DTB 构建，缺 `/chosen/rng-seed`，启动即 panic `secure Wi-Fi startup entropy failed`，已替换，勿用旧哈希 `2dd64187…`） |
| `sg2002_starryos_wifi_sta_prechain_20260929.img` | `a6633f3f6` + 工作树未提交 | 同上 | `www/sg2002/wifi-sta/lcn-sta-defer0.dtb` | 已构建自检，待板测（第四阶段·完成前交帧：owner 步内顺序 + 取消语义声明；与 `…_attrib2_20260928.img` **只差内核**）；SHA-256 `4ed9d8d8c377f6bf52dcbc4812a471ec3e3d435522aae4569505b63065adc873`（**审查修复后重建，以本行为准**；修复前的同回合同名镜像哈希为 `8911f913…`，勿用） |
| `sg2002_starryos_wifi_sta_attrib3_20260929.img` | `a6633f3f6` + 工作树未提交 | 同上 | `www/sg2002/wifi-sta/lcn-sta-defer0.dtb` | 已板测（第四阶段·归因探针深化：park 唤醒来源 + `chan_*` 信道占用 + 栈侧供帧深度/间隔；与 `…_attrib2_20260928.img` **只差内核**）；SHA-256 `8867305d3e2049547431f45fec52d4ca434555015372c96beb6bbf51dcf9cef1` |
| `sg2002_starryos_wifi_sta_instr_20260930.img` | `a6633f3f6` + 工作树未提交 | 同上 | `www/sg2002/wifi-sta/lcn-sta-ahead1.dtb` | 已构建自检，待板测（第五阶段第二轮：S0 仪器 + S1 后继成形上限；基座 `…_ahead1_20260929.img`，**只差内核**）；SHA-256 `5292431dac05d138110ebc07965a02feaffde02b08460bd2cdeb5c5f1238bb40` |
- 入库板级 DTB（`os/StarryOS/configs/board/licheerv-nano-sg2002.dtb`）在提交 `22e0890f0` 中带上 `aic,tx-aggregation=32` / `aic,tx-aggregate-bytes=49152`；它服务走板级配置的构建，**不作为板测镜像的 DTB 输入**（板测镜像必须用带 `rng-seed` 的 STA 血统设备树）。

- **镜像必须用 STA 血统的设备树**（`www/sg2002/wifi-sta/lcn-sta-*.dtb`）：它带 `/chosen/rng-seed`，而入库板级 DTB 没有；缺该属性时内核会在网络队列初始化处 panic （`secure Wi-Fi startup entropy failed: trusted wireless connection entropy is unavailable`）。
- 构建时 HEAD 早于内容提交的两个镜像（P0a、HT40/SGI）用「内容对应的提交」一列表示其代码内容；这两个镜像是从同一工作树状态分别构建的，提交顺序为 `fd80b2b69` → `752f308e0`。
- 镜像文件名、SHA-256 与原始日志按轮记录在各自小节；镜像本体与 `.json` 在本地构建产物目录，不入库。
- **2026-09-25 迁移**：dev 由 `9a7b868ba` 更新到 `714accd8f`，此前散在 `probe/aic8800-*` 上的工作
  已全部变基到新 dev 并落在 `sg2002/wifi-opt` 上。6 个提交逐字节等价（patch-id 一致），哈希随基线变化：

| 内容 | 迁移前（`probe/aic8800-supply`） | 迁移后（`sg2002/wifi-opt`） |
| --- | --- | --- |
| credit 本地记账（周期 1） | `77cc80819` | `0d12a9c85` |
| TX/RX 供给探针（周期 P3） | `c76bb121f` | `f1bff4ce9` |
| 写成本探针（周期 P4） | `8ba16f717` | `0e3bf2c4c` |
| 最小聚合（周期 P5） | `89ff23e4f` | `9b0b505b7` |
| 完成事件修正（周期 P5 审查） | `b677b55b1` | `a9bdf9ce8` |
| 帧流布局与 token 归还修复（周期 P6） | `f870ea207` | `a8d6fca2e` |

- **历史快照分支**（不再更新，保留用于回看当时的探针形态与哈希）：`probe/aic8800-tx-timing`
  （`086c8eec9`，周期 M 探针第一代）、`probe/aic8800-pipeline`（`d1cd0a4c3`，周期 P2 窗口化探针）、
  `probe/aic8800-credit-accounting`（`f167244b4`，周期 1 板测时叠了探针的临时分支）。
  这些分支上的探针代码已被当前分支的新一代取代，周期记录中出现的旧哈希均属这一类。

## 周期 M：探针（阶段 M）

### 改动

临时诊断插桩，不改变任何数据面行为：

- `drivers/net/aic8800/src/device/probe.rs`：新增计数器与整点上报（每 2000 包一行 `[wifi-probe]`）；
- `data_plane.rs` / `progress.rs` / `owner.rs` 打点：credit 读数、回退次数与等待、CMD52/CMD53 往返、
  相邻 CMD53 完成间隔、入队时核心 TX 队列是否已有积压。

分支 `probe/aic8800-tx-timing`（提交 `086c8eec9`，基于 dev `9a7b868ba`；历史快照分支，见「分支与提交」）。

### 测试

- 镜像：`sg2002_starryos_wifi_sta_probe_20260924.img`（内核 dev `9a7b868ba` + 探针，STA 编译期凭据）；
- 板端 STA 连 PC 热点，iperf3 跑下行、纯上行、双向三轮，采集串口探针行。

### 现象

详见 `probe-round1-20260924.md` 与原始日志 `probe-round1-20260924.log`：

- credit 常态远高于门限（均值 75–119），逐包读寄存器属结构性浪费；
- 一次 CMD52 往返 62–75 µs，其中纯总线约 5 µs，其余是中断/唤醒/owner 轮转开销；
- 回退一次实测 1.36–1.40 ms，比设定的 1 ms 多约 0.33 ms 的到期唤醒开销；
- 每包周期 1.2–1.5 ms，可归因部分不足一半，650–800 µs 无法归因（阶段 2 的目标）；
- 双向用例跑完后板端 shell 失去响应（网络仍可用），与 09-17 记录的「多流 / UDP 卡死」同类，
  与插桩无关，需单独排查。

### 对方案的影响

- 阶段 1 的 §5.1、§5.2 两项前提成立，折算合计约 15%，按原计划先做；
- 阶段 2 的收益形状需要重估（原分析文档「L ≈ 20–65 µs」比实测小一个量级）；
- 追加「第二轮探针」需求：把事务之外的 650–800 µs 按窗口拆开（CMD52 完成→CMD53 发出、
  CMD53 完成→下次 CMD52 发出、适配层准备耗时）。

## 周期 1：credit 本地记账（阶段 1）

### 改动

`drivers/net/aic8800`：

- `DataPlaneState` 新增 `tx_credits: Option<u8>`：一次寄存器读取授权的包缓冲数，减去此后完成的写入数；
- `drive_ready()`：缓存值高于命令保留量时跳过 `TransmitFlow`，直接发 CMD53；
- `consume_transmit_data()`：每完成一次写入扣一个；降到保留量即丢弃缓存，下次发送重新读取；
- 失效条件：命令转发（V3 命令与数据共用同一数据 FIFO）、生命周期命令、取消；
- `IO_RETRY` 1 ms → 200 µs（与固件排空一包的时间同量级）。

分支 `sg2002/wifi-opt`，提交 `0d12a9c85`（迁移前为 `probe/aic8800-supply` 上的 `77cc80819`），
同一提交内同步了 `docs/design/unified-sdio-aic8800.md` 的 credit 与退避描述。
本轮板测用的镜像把探针叠在该提交之上（临时分支 `probe/aic8800-credit-accounting`，提交 `f167244b4`），
板测完成后探针不进入正式分支。

### 测试

- 单测：`cargo xtask test --since dev` 通过（141 个 lib 测试），其中新增
  「一次 credit 读取连发多包、缓存逐包递减」与「命令转发清空缓存」两条；
  既有的保留量门限与回退测试同步复核；
- 静态检查：`cargo xtask clippy --package aic8800` 通过（base / rdif / host-test 三组）；
- 板测：`sg2002_starryos_wifi_sta_credit_20260924.img`（基底为周期 M 的探针镜像，
  仅内核换成本周期版本），用例与周期 M 相同。

本轮镜像的构建口径（可复现）：

```bash
# 内核：必须用带 aic8800-wifi 的板级配置，否则编出的内核不含 WiFi 驱动（体积少约 700 KB）
STARRY_WIFI_SSID=aasta STARRY_WIFI_PASSWORD=12345678 \
  cargo xtask starry build -c os/StarryOS/configs/board/licheerv-nano-sg2002-wifi.toml
# 镜像：在上一轮镜像上只换内核 + STA DTB
sg2002-image-build.sh update-kernel <上一轮镜像.img> \
  --kernel target/riscv64gc-unknown-none-elf/release/starryos.bin \
  --dtb www/sg2002/wifi-sta/licheerv-nano-sg2002-sta.dtb -o <本轮镜像.img>
```

镜像自检：FIT 内 kernel 的 crc32 `6f00612c` 与 `target/.../release/starryos.bin` 一致，
rootfs 内 `/starryos.uimg` 同步为新内核，FIT 默认配置 `config-sg2002_licheervnano_sd`、
Load 地址 `0x80200000`。

### 现象

原始日志：`stage1-board-20260924.log`（板端串口，29 条探针行）。口径与周期 M 相同：
PC 作 iperf3 client、板端作 server，`-R` 为上行、`--bidir` 为双向。

吞吐对照（同口径）：

| 用例 | 周期 M | 周期 1（本轮） | 变化 |
| --- | --- | --- | --- |
| 下行 PC→板 | 16.7 / 25.1 Mbps | 18.2 / 22.3 / 25.3 Mbps | 持平（波动范围内） |
| 上行 板→PC | 6.09 Mbps | **9.09 Mbps** | +49% |
| 双向 板→PC | 8.59 Mbps | 9.45 Mbps | +10% |
| 双向 PC→板 | 5.54 Mbps | 6.73 Mbps | +21% |
| 上行重传 | — | 0 | 未见超发导致的静默丢包 |

（09-17 基线的 6.61 / 7.30 Mbps 是板端作 client 的旧口径，只能作方向性参考。）

探针按 2000 包窗口差分：

| 指标 | 周期 M | 周期 1 |
| --- | --- | --- |
| CMD52 次数/包（常态窗口） | 1.00 | **0.008 ~ 0.011** |
| 每次读数的授权包数 | — | 85 ~ 100（读数 86~103） |
| 每窗口回退次数 | 173 ~ 2242 | 绝大多数为 **0**，池见底时 188 ~ 1496 |
| 回退等待均值 | 1363 ~ 1404 µs | **529 ~ 534 µs** |
| 上行 TX 包速率 | 674 包/秒 | **~790 包/秒**（735~916） |
| txq 有积压比例 | 75% | 66% |
| 上行 CMD53 边际往返 | ~490 µs | **~720 µs**（见发现 2） |

结论：

1. 两项机制都按设计生效：缓存命中时一次读数连发 85~100 包（CMD52 降到约 1/100），
   回退等待从 1.4 ms 降到 0.53 ms，与「IO_RETRY=200 µs + 约 0.33 ms 唤醒开销」的预估一致。
2. 上行 TX 包速率 +17%，与方案 §5 预估的约 15% 吻合；同口径上行 TCP 6.09 → 9.09 Mbps。
3. RX 未劣化（下行持平、双向 PC→板上升），无重传，满足方案 §10.2「RX 劣化即回退」的前提。
4. 板端**仍然会卡死**，且本轮发生在上层：卡死点在本轮 `--bidir` 用例**之前**，
   终端失去响应、Ctrl-C 退不出；而网络面依旧可用（`--bidir` 随后照常跑完并出结果，
   日志里 iperf3 进程都正常退出、板端仍在监听）。与周期 M 的现象同类，
   初步判为上层（shell / 终端 / 进程管理、smoltcp）问题，**暂不纳入本优化，保持记录**。
   因此本轮数据不足以判断回退缩短是否与该现象有关。

### 两个需要跟进的发现

**发现 1：credit 池见底后缓存退化为老行为（安全，但无收益）。**
t≈153–158 s 的一次瞬态里读数掉到 2~25：单窗口 1496 次回退、CMD52/包升到 0.81。
此时包速率反而更高（679~836 包/秒），说明池低是**空口已饱和的表象**而非驱动限流；
缓存没有高估（读数低时照旧重读与回退），方案 §11 R3 的风险未发生。
瞬态之后读数回到 86~103、CMD52/包回到 0.011。

**发现 2：CMD53 边际往返从约 490 µs 涨到约 720 µs（+200 µs），但每包周期仍从 1.48 ms 降到 1.27 ms。**
候选解释（待 probe v2 区分）：

- 提交点变化：缓存命中时 CMD53 由 `drive_ready()` 发出，而旧路径的 CMD53 是紧跟 CMD52 完成、
  在同一次 `advance()` 里发出的（经 `io.next`），现在多了一次「回到 runtime / rearm 边界」的往返；
- 低配额下的固件行为：连发到配额边界后，最后一笔写入要等固件腾出缓冲才完成。

两者都指向阶段 2 要动的位置：每包仍有约 550~590 µs 落在事务之内、总线之外
（1536 B 的总线时间只有 131 µs）。

### 判据对照（方案 §1.2）

| 指标 | 目标 | 本轮 | 差距 |
| --- | --- | --- | --- |
| TX TCP 板→PC | ≥ 40 Mbps | 9.09 Mbps | 约 4.4 倍 |
| 每包周期 | ≤ 0.25 ms | 1.27 ms | 约 5 倍 |
| SDIO 事务/包 | 1（CMD53） | 1.01 | **达标** |

阶段 1 按方案只占其中一小块，剩余差距是阶段 2/3/4 的目标。

## 周期 P2：窗口化探针

### 改动

`drivers/net/aic8800/src/device/probe.rs` 重写为窗口化统计（每 2000 包或静默 2 s 出一行，
出完即清零），并按周期 1 的两个发现分桶：

- `c53 full / small`：CMD53 往返按线上帧长分开（≤512 与 >512），排除周期 1 的「边际往返 +200 µs」
  是帧长构成差异造成的假信号；
- `cache fresh / hit`：按「紧跟一次信用读取的写入」与「用缓存配额写入」分开；
- `gap ready / idle`：上次完成→本次发出的间隔，按「完成时核心队列里是否已有帧」分桶——
  前者是 owner / runtime 调度开销（阶段 2 能消掉），后者是在等上游供帧（阶段 2 消不掉）；
- `steps per_pkt`：owner 步数摊到每包，给调度开销一个上界。

分支 `probe/aic8800-pipeline`（提交 `d1cd0a4c3`，叠在周期 1 提交之上；历史快照分支，见「分支与提交」）。
镜像 `sg2002_starryos_wifi_sta_pipeprobe_20260924.img`（基底为周期 1 镜像，仅换内核），
sha256 `1656c48b1452118c5806e56307480fb395325a48e3b66c0ad16ca3cd3ef0a8d4`。

### 测试

与周期 1 相同的用例（PC 作 client、板端 server，`-R` 上行 + `--bidir` 双向）。

### 现象

原始日志：`probev2-board-20260924.log`（板端串口，窗口化探针）。本轮三个用例：
下行 TCP 24.6 Mbps、上行 TCP 3.56 Mbps、双向板端 TX 9.07 / RX 4.44 Mbps。
字节核对：探针窗口累加的包数 × 1414 B 与 iperf3 报告一致（上行 3.6 vs 3.56 Mbps、
双向 9.7 vs 9.07 Mbps），说明探针计数完整、可信。

按用例分类的窗口数据（每 2 s 一窗）：

| 用例 | 帧长构成 | 包/秒 | CMD53 往返 | gap ready | gap idle | txq 积压 | 步/包 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 下行（板端只在发 TCP ACK） | 几乎全 small | 180 ~ 370 | small ≈ 125–153 µs | 主导（400~580/窗，均值 640~1100 µs） | 60~150/窗，均值 9.5~24 ms | 78 ~ 87% | 11 ~ 15 |
| 上行（PC 只收） | 99.9% full | **150 ~ 450** | full ≈ 619–710 µs | 14~319/窗 | **主导（282~634/窗，均值 2~7.5 ms）** | **4 ~ 38%** | 5 ~ 6 |
| 双向 | ~95% full | **780 ~ 862** | full ≈ 648–720 µs | **主导（1479~1693/窗，均值 466~614 µs）** | ≈0 | **95 ~ 100%** | 5 ~ 6 |

### 结论

**1. 周期 1 的「CMD53 边际往返 +200 µs」结案：是帧长构成差异，不是回归。**
本轮 `full` 帧往返 648~720 µs，与周期 1 同口径下算得的 590~735 µs 一致；
`small` 帧只有 116~155 µs。周期 M 的上行窗口平均帧长 1127 B（full/small 混合），
按本轮两个桶折算恰好落在其实测的 490~516 µs 上——之前那个「+200 µs」是拿混合帧长的均值
与全满帧的均值对比造成的假信号。

**2. 纯上行用例里驱动侧 TX 通路是空闲的——但「谁在限速」尚未定位（见周期 P3）。**
上行窗口里核心 TX 队列在完成时刻**有 90% 是空的**（积压率 4~38%）、gap 被 `idle` 主导
（均值 2~7.5 ms）——即事务通路在等帧，而不是总线或事务本身受限。
但**不能由此断定是 TCP 栈本身慢**：旧设计（PR #1914 那代、`fdrv` 带专用 TX 线程）
在同一块 D80 上跑出过 10~14 Mbps 上行，说明栈有能力供得上；本轮 3.56 Mbps
（周期 1：9.09、周期 M：6.09）更可能是**供给与唤醒链**的问题——
从 smoltcp 提交（`TxNotify::Deferred`，允许推迟到 flush）到 rdif 队列、
再由 owner 在 `advance_with_cause` 里 `take_tx_frame()` 拉进核心，中间每一次往返
都要等调度。本轮的核心队列采样无法区分「栈没产帧」与「帧在上一层等我们拉」，
这个区分放到周期 P3。
**驱动受限的用例是双向**：队列积压 95~100%、gap 被 `ready` 主导（说明完成时手里总有帧）。

**3. 阶段 2 的抓手被证实（这是本轮最重要的收获）。**
在驱动受限的双向用例里，每包时间可完整拆开：

| 构成 | 量级 | 归属 |
| --- | --- | --- |
| CMD53 往返（发出→完成） | ≈ 680 µs（1536 B 总线时间仅 131 µs） | 阶段 3 压非总线部分 + 阶段 2 部分重叠 |
| 完成→下一笔发出（gap） | ≈ 540 µs，`ready` 主导 | **阶段 2「完成即续发」的直接目标** |
| 合计 | ≈ 1.22 ms/包（860 包/秒） | 判据 ≤0.25 ms/包 仍差约 5 倍 |

gap 与 CMD53 往返里各有约 5~6 次 owner 轮转（`steps per_pkt` 5~6），
按 gap/步数折算约 100 µs/步——与周期 M 实测的「唤醒开销约 70 µs」同量级，
说明这个 gap 就是「完成 → 回到 runtime → 再被我方调度到」的往返，正是阶段 2 要消掉的对象。

**4. `cache fresh / hit` 无稳定差异**（fresh 每窗仅 3~8 个样本，均值在 424~827 µs 间跳），
不支持「同一 `advance()` 内发出的 CMD53 更快」这一猜想，故不作为结论。
`backoff` 全程为 0，credit 读数 51~132、每窗仅 3~8 次（约 1/100~1/200 包），
周期 1 的缓存策略在本轮负载下依旧成立。

## 周期 P3：供给与总线占用探针

### 改动

本轮不改数据面行为，只加探针。分支 `sg2002/wifi-opt`（提交 `f1bff4ce9`，叠在周期 1 提交之上）。

**要解决的问题**：周期 P2 的「gap 由 owner 往返构成」是由「gap ÷ 步数 ≈ 100 µs/步」折算出来的间接结论。
按当前代码，同一个 gap 至少还有两种解释，而它们对应完全不同的下一步：

1. **RX 事务占用同一条总线**：核心准入的优先级是 mailbox > 优先事件 > RX 扫描 > TX
   （`device/data_plane.rs` 的 `drive_ready`），CARD_INT 又是水平触发；一个 RX 帧要走
   `ReceiveCount` →（`ReceiveByteLength`）→ `ReceiveData` → 回到 `ReceiveCount` 的链路
   （同文件 `drive_receive_scan` / `consume_receive_*`），这些事务与 TX 写在同一个 owner、
   同一条 SDIO 总线上严格串行。双向用例里 PC→板的流量正落在 RX 路径上。
2. **owner 被调度门控**：TX 完成事件按 `#2299` 的设计必须退回 rearm 边界
   （`rdif/owner/progress.rs` 的 `CardIrqWait::complete_event`），而 owner 只在 `finish_idle()`
   且 `begin_rearm()` 成功时推进（`net/ax-net/.../executor/mod.rs`、`queue_runtime/state.rs`）；
   若期间有新的 IRQ 把组状态置为 MISSED，这次推进整体跳过。
3. **软件往返**（周期 P2 的原始结论）：完成 → 回到 runtime → 再被调度到。

探针按这三条分别布点：

- **设备侧**（`drivers/net/aic8800/src/device/probe.rs` 重写）：事务按类别记账
  （TX 写 / credit 读 / RX 数据 / RX 控制 / 其它）的次数、平均与最大往返；RX 读按长度分桶；
  写完成时的核心队列与 RDIF 环深度；gap 按「窗口内 RX 事务数」与「上一笔写完成时下一帧在哪一层」
  两个维度分别分桶；周期直方图；credit、回退、owner 步数与调用次数沿用前轮口径。
- **执行器侧**（`net/ax-net/src/queue_runtime/executor/probe.rs`，本轮新增、临时）：每 2 s 一个窗口，
  统计循环轮数、`poll` 次数与耗时、阻塞等待次数与耗时、owner 调用次数与耗时（含 >200 µs 计数）、
  协议侧待提交帧数（`tx_ready` 深度）、TX 提交 / 回收与 RX 回收次数、IRQ 次数。

两侧窗口同为 2 s，且都用 `monotonic_time_nanos` 采样，串口日志可直接对照。

### 测试

- 内核：`licheerv-nano-sg2002-wifi.toml` + STA 编译期凭据；
- 镜像：`sg2002_starryos_wifi_sta_supplyprobe_20260925.img`（基底为周期 P2 镜像，仅换内核 + STA DTB），
  内核 sha256 `c8f609be3dc05708eb6c6b6739ad6abdef6759564c8fbec876ebb9a63387cf40`；
- 用例：与周期 P2 相同（PC 作 iperf3 client、板端 server），下行 / 上行 `-R` / 双向 `--bidir`；
  探针行 `[wifi-probe]`、`[wifi-probe-rx]`、`[netprobe]`。

本轮不改行为，吞吐应与周期 1 / P2 持平；明显偏离即说明探针自身开销过大，需重测。

### 现象

原始日志：`probev3-board-20260925.log`（板端串口，含 82 条 `[wifi-probe]`、83 条 `[wifi-probe-rx]`、83 条 `[netprobe]`）。
本轮四个用例（PC 作 iperf3 client、板端 server）：下行两次 26.7 / 28.6 Mbps（板端作 receiver）、
上行 `-R` 9.50 Mbps（板端作 sender）、双向 `--bidir` 板端 TX 9.64 / RX 5.77 Mbps。
下表取各用例的稳态窗口（每窗 2 s，共 14~16 窗）平均；`[netprobe]` 的占比按窗口时长折算。

| 指标 | 下行（板端 RX 28.6 Mbps） | 上行（板端 TX 9.50） | 双向（板端 TX 9.64） |
| --- | --- | --- | --- |
| TX 写次数 | 260 /s | 811 /s | 890 /s |
| TX 写平均往返 | **148 µs**（每笔 512 B） | **700 µs**（每笔 1532 B） | **679 µs**（每笔 1456 B） |
| 每包周期 | —（ACK 流） | 1233 µs | 1330 µs |
| gap（窗口内无 RX 事务） | 182 µs，占 84% | 81 µs，占 78% | 74 µs，占 72% |
| gap（窗口内有 RX 扫描） | 24.3 ms，占 16% | 2.09 ms，占 22% | 2.13 ms，占 28% |
| RX 读次数 / 平均往返 | 374 /s，1238 µs（每读 10.1 KB） | 650 /s，195 µs（每读 516 B） | 598 /s，374 µs（每读 1.9 KB） |
| RX 控制事务 | 467 /s，262 µs | 1044 /s，61 µs | 854 /s，96 µs |
| 写完成时下一帧在哪 | core 83% / ring 0.7% / 无 16% | core 12% / **ring 88%** / 无 0.03% | **core 99%** / ring 1% / 无 0.07% |
| 执行器：阻塞等待 / 轮询 / owner | 72% / 4% / 21% | 64% / 4% / 26% | 60% / 5% / 28% |
| owner 单次耗时 | 98 µs | 55 µs | 63 µs |

### 结论

**1. 周期 P2 的「gap ≈540 µs 是 owner 往返」不成立——阶段 2 的收益上限只有约 6%。**
gap 是双峰分布，不是单峰：窗口内没有 RX 事务时只有 **74~81 µs**（72~78% 的包），
有 RX 扫描时是 **2.1~2.4 ms**（22~28%）。P2 的 540 µs 是这两者的加权平均。
「完成即续发」能消掉的只是那 74~81 µs，占每包周期的 6% 左右。

**2. 纯上行不是「上游没供帧」，是驱动侧限速。**
写完成时刻有 **88% 的样本 RDIF 环里有帧**（核心队列空、环里有帧），两层都空的样本只有 6 个（0.03%）。
也就是说帧早已由协议交给驱动、在环里等 owner 拉取——周期 P3 原本要区分的两个假设，「帧在上一层等 owner 拉」成立。
P2 的「纯上行受上层供帧限制」应更正为：受**驱动自己的每包周期**限制。

**3. 主要成本是 CMD53 写本身，且它随帧长剧增；读方向则接近总线速率。**
写：512 B → 148 µs，1456~1532 B → 679~700 µs（跨用例比较，每多一个 512 B 块约 +220 µs）。
读：512 B → 195~262 µs，1.9~10 KB → 374~1238 µs，边际约 **8~10 MB/s**，接近 4-bit / 25 MHz 的总线上限（12.5 MB/s）。
即：**读方向已经是「多字节摊薄固定开销」的形态**（一次读常带 2~4 个帧，下行每读 10 KB），
而**写方向是一包一笔事务**，每字节成本是读方向的 3~6 倍。周期 P2 记录的下行 26.7 Mbps 正是读方向的成绩。

**4. 软件不是瓶颈。** 执行器 60~72% 的时间在阻塞等待（每次 200~490 µs），owner 只占 21~28%，
poll 占 4~5%；`skipped`（owner 推进被 MISSED 跳过）每窗只有个位数到十几。因此「唤醒/调度」不是主因。

**5. RX 与 TX 共用同一条总线，且 RX 里有相当一部分是我们自己 TX 的代价。**
上行用例里每 2 s 有约 1300 次 ≤512 B 的读（195 µs 一次），但只交付约 340 个数据帧给协议——
其余是固件对**每个已发数据包**回的确认（`ParsedFrame::DataConfirmation`）与 TCP ACK。
双向用例里 RX 事务（读 + 控制）合计约 1450 次/2 s。这些事务与 TX 写在同一个 owner 上串行，
并直接构成 TX 的 gap。

**6. 测量口径的两点说明（供解读用）**
- `rx1` 桶恒为 0：一次 RX 扫描至少是「读计数 + 读数据」两笔事务，所以 bucket 只有 0 与 ≥2。
- RX 读的 `bytes` 是**块补齐后**的传输长度（一个 60 B 的 ACK 也按 512 B 读），
  所以不能用它与 iperf3 的字节数直接对齐；字节对齐要用 `frames` 与 iperf3 的包数。

### 对方案的修订

- 方案 §6（阶段 2 把准备移出关键路径）：前提是本轮的 gap 主要是软件往返。实测 gap 的空闲部分是 74~81 µs，
  收益上限约 6%，**不作为下一步的主线**；§6 的暂存与「等待窗口内做准备」仍可留作后续小项。
- 方案 §8（聚合，原列「按需」）：现在是**唯一能同时摊薄写方向固定开销与 RX 确认开销**的改动，
  与厂商 32 包/次 CMD53 的做法一致，优先级上调。
- 方案 §7（关键路径清理）：每笔事务省下的软件时间对 4800 笔/2 s 的事务量仍有意义，作为并行小项。
- 方案 §3（空口速率）：本轮下行 26.7 Mbps 说明空口在 RX 方向至少能跑 27 Mbps；
  上行 9.5 Mbps 的差距目前归因到 SDIO 写方向，空口不是首要怀疑对象（待 P4 确认写的长度依赖）。

## 周期 P4：写成本探针

### 改动

仍不改行为，只在 P3 探针上补三处（分支 `sg2002/wifi-opt`，提交 `0e3bf2c4c`）：

- **写按线上帧长分桶**（1 / 2 / 3 / 更多块）。P3 的「512 B 148 µs 对 1536 B 700 µs」是跨用例比较，
  而两个用例的空中队列状态不同，存在混淆；桶内比较可以在**同一个窗口**里把长度效应单独拿出来。
- **事务跨几个 owner 轮次**（emit 与 complete 时的 `OWNER_CALLS` 之差），以及每类事务的**最小**往返。
- **执行器侧**：`schedule_irq` 记录 IRQ 时刻，owner 每次推进记录「距上次 IRQ 多久」（唤醒延迟直方图）；
  阻塞等待按「通知唤醒 / 到期」分开，并给出等待时长直方图。

### 测试

- 镜像：`sg2002_starryos_wifi_sta_writecost_20260925.img`（基底同 P3，仅换内核 + STA DTB），
  内核 sha256 `03c0a2e7584b4e0400d09a112f3f3488f80d90b9135682689348cd9863e2b51f`；
- 用例：与 P3 相同（下行 / 上行 / 双向），重点读双向窗口里的写长度分桶。

### 现象

原始日志：`probev4-board-20260925.log`。本轮九个用例，下表取稳态窗口（每窗 2 s）平均。
其中双向 30 s 用例（板端 TX 9.69 / RX 4.78 Mbps）与上行 30 s 用例（板端 TX 8.42 Mbps）为主：

| 指标 | 上行（板端 TX 8.42） | 双向（板端 TX 9.69 / RX 4.78） |
| --- | --- | --- |
| 写 512 B（1 块） | 152 µs（8 个样本） | **127 µs**（255 个样本） |
| 写 1024 B（2 块） | 550~728 µs | 526 µs |
| 写 1536 B（3 块） | **706~780 µs** | **759 µs**（1536 个样本） |
| 写往返最小 | 126~238 µs | **101 µs** |
| 事务跨 owner 轮次 | 1.98 | 1.98 |
| 每包周期 | 1393~1442 µs | 1097 µs |
| gap 无 RX / 有 RX | 79~89 µs（78%）/ 1855~10893 µs（22%） | 67 µs（72%）/ 1214 µs（28%） |
| 写完成时下一帧 | core 7% / ring 89% / 无 0 | core 95% / ring 5% / 无 0.1% |
| 阻塞等待 | 216~225 µs，**100% 通知唤醒、0 次到期** | 212 µs，100% 通知 |
| 中断→owner 唤醒 | 229~243 µs | 243 µs |
| owner 单次耗时 | 56~58 µs | 63 µs |

失败的那次双向用例（板端 RX 0 Bytes、TX 181 Kbps）：探针显示驱动几乎空闲
（每 2 s 只写 2~4 帧、周期约 1.07 s、无错误、1 块写 156~254 µs），是上层连接停滞，与驱动无关。

### 结论

**1. 写的成本由传输长度决定，与空中队列状态无关。** 同一个双向窗口里
512 B 写 127 µs、1536 B 写 759 µs（两者在同一时刻、同一空口条件下发出）；
上行用例里 1 块 152 µs、3 块 706~780 µs。「固件按空口队列节奏完成 SDIO 写」的假设被否定。
同时 3 块写的最小值只有 101~238 µs，说明 ~780 µs 不是硬性下限，而是**常态而非物理极限**。

**2. 两个方向都是「小传输贵、大传输便宜」，而 RX 已经站在便宜的一侧。**
按本轮与周期 P3 的合并读数（每 512 B 折算）：

| 传输 | 1 块 | 2~4 块 | 5~16 块 | >16 块 |
| --- | --- | --- | --- | --- |
| 读 | 195~262 µs | 519~815 µs | ~999 µs | ~2166 µs |
| 写 | 127~152 µs | 526~728 µs | 未测 | 未测 |

读的边际速率随长度升到约 10 MB/s（≈ 4-bit / 25 MHz 的 12.5 MB/s 上限），
写只测到 3 块、仍停在每块数百微秒的慢档。驱动侧 RX 之所以能跑 26~28 Mbps，
正是因为固件成批给帧（一次读常带 2~4 帧、下行每读 10 KB）——**RX 已经在快档，TX 是一包一笔事务、从未进入快档**。

**3. 软件依旧不是瓶颈。** 阻塞等待 100% 由通知（中断）结束、没有任何一次到期超时；
等待直方图呈两簇（<50 µs 与 250~1000 µs），后者对应多块传输在设备里的时间。
owner 单次 56~63 µs、中断到 owner 的唤醒 229~247 µs——都在可解释的量级，不构成主因。

**4. 供给不是问题。** 双向用例里写完成时刻 95% 核心队列已有帧；上行用例里 89% 的情况帧在
RDIF 环里等着拉。两个用例都不缺帧。

### 判读结论与下一步

按上板前记下的判读表，读数落在第一行：**写成本由长度决定**。据此：

- **阶段 4（聚合）立项**：把每笔 CMD53 从 1 包提到 K 包，让写方向也进入「每块数十微秒」的快档。
  依据是读方向已经证明同一张卡在长传输下能到 ~10 MB/s，而写方向从未试过超过 3 块。
  预期收益 2~4 倍（K=4~32），上限是总线（~10 MB/s 线速率）与空口速率（方案 §3，待并行确认）。
- **方案 §3（空口速率）保持并行议题**：本轮写往返随长度的斜率折算出空口有效速率约 18 Mbps
  （同类链路上 PC→板方向实测 27 Mbps），若写完成确实与空口相关，聚合与空口速率会叠加。
  该议题需要能读出实际 MCS 的手段（厂商镜像或固件消息），本轮未解决。
- **「唤醒/调度」不立项**：等待 100% 是通知驱动，没有到期超时，软件侧没有大漏损。

## 周期 P5：最小聚合（每笔 CMD53 带 4 包）

### 改动

周期 P4 的读数指向唯一杠杆：写方向的每块成本远高于读方向，而读方向已经因为固件成批给帧而站在快档。
本轮让写方向也成批：**一笔 CMD53 携带最多 4 个完整线上帧**。

分支 `sg2002/wifi-opt`，提交 `9b0b505b7`。改动点：

| 位置 | 改动 |
| --- | --- |
| `rdif/owner/progress.rs` | `submit_one_tx` 一次从 RDIF 环取最多 4 帧，作为 `AicInputEvent::TxBatch` 一次交给核心；`initialize_device` 用 `set_tx_aggregation(4)` 打开批量 |
| `device/model.rs` | 新增 `AicInputEvent::TxBatch`（单帧的 `Tx` 保留，宿主测试沿用） |
| `device/data_plane.rs` | `prepare_next_transmit` 在首帧之后继续追加队列里的帧（`extend_active_write`），上限由 `aggregate_limit()` 给出；`consume_transmit_flow` 在读到 credit 后再补一次（读数到手才知道固件还有几个缓冲） |
| `device/owner.rs` | `ActiveTx` 增加 `extra_tokens`；`packets()` 给出本笔写携带的包数；新增被事件队列挡下的完成回退队列 |
| `device/data_plane.rs` | `consume_transmit_data` 按包扣 credit、按包回 token |
| `rdif/owner/output.rs` | 完成回环改成三态（`Published` / `Deferred` / `Waiting`），被挡下的 token 留在待回队列里重试 |

**边界**：`aggregate_limit()` = min(层给的 K, 缓存 credit − 命令保留量)；没有 credit 读数时只能带 1 包
（固件还剩几个缓冲必须先知道），所以每轮 credit 读数之后的第一笔写仍是单包，之后才是成批的。
暂停、取消、失败路径都按 token 逐个回还；RX 优先（`#2299`）的准入顺序未改。

**顺带修掉的一处隐患**：完成回环原先在 `tx_complete` 环满时会把事件吃掉、缓冲却留在 `tx_tokens` 里，
单包时窗口极小，成批之后一次回来 4 个就可能滞留。现在被挡下的 token 会留在待回队列、每次 `flush` 重试。

### 测试

- 宿主测试：新增 `one_write_carries_a_packet_batch_and_completes_every_token`——同一设备上先量单帧写的长度，
  再让 4 包一次入队，断言写长度是 4 倍、完成后按序回 4 个 token、credit 恰好扣 4；
  既有 121 条测试全部通过（默认 K=1，行为与改动前逐字一致）；
- `cargo fmt`、`cargo xtask clippy --package aic8800` 通过；
- 镜像：`sg2002_starryos_wifi_sta_aggr4_20260925.img`（基底同 P4，仅换内核 + STA DTB），
  内核 sha256 `e4b181c77e78b220f4d3ff5da29f20fef35d5b124e769f3a9af62c13c62a1ae4`；
- 用例：上行 `-R` 与双向 `--bidir` 各 30 s（下行只作参考，板端几乎不发数据帧）。

### 现象

原始日志：`probev5-board-20260925.log`；审查结论：`probev5-aggregation-review-20260925.md`。

- **写事务层面的聚合生效**：10+ 块（4 包）写的往返均值为 1102 µs，写次数随之下落；
  该读数只说明**事务**变快，不能按包摊薄（每笔只有第 1 帧存活时，每帧成本为 1102 µs，
  反高于周期 P4 的 706~780 µs）。
- **但四个 TX 方向用例都停住**：每次恰好传出 256 KB——该数值是发送侧**应用写入量**，不是送达量
  （上行 99.9 Kbps / 524 Kbps / 210 Kbps，双向板端 TX 80.7 Kbps），下行不受影响（板端接收 22.3 Mbps）。
- 停住时驱动侧读数：`tx_submit == tx_done`（无缓冲失衡）、无任何内核或驱动错误行。
  原「协议层待交帧队列深度 99% 为 0 ⇒ 帧没到驱动」的推理已作废：该深度采样于提交循环排空之后，
  健康窗口同样近乎全 0。
- 判读表第四行（写路径停滞）出现；停住与聚合的关系由 P6 的审查与修复给出（见下）。

### 后续

由周期 P6 的审查与修复给出：审查确认了聚合写的线上布局缺陷（见 P6 改动），
修复后重测以闭合因果链；K=1 对照镜像降为「修复后仍停住」时的备选手段。
判据见 `probev5-aggregation-review-20260925.md` 第 7 节。

### 判读口径（上板前记下的，保留备查）

| 读数 | 含义 | 下一步方向 |
| --- | --- | --- |
| `size blk` 分桶出现 4-6 块、且该桶的每包成本明显低于 3 块桶（如 300 µs 对 700 µs） | 写方向确实能进快档 | 聚合成立：正式分支上把 K 做成可配置、补冲刷策略与单测，并按方案 §8 立项 |
| 4-6 块桶的每包成本与 3 块桶接近 | 每块成本是卡的固有属性，长事务不摊薄 | 聚合不成立，回退这一改动；转向空口速率（§3）与每笔事务的软件成本（§7） |
| 包速率/吞吐随 K 提升，但双向用例的 RX 明显下降 | 长事务挤压了同总线的接收 | 降 K，或给写方向加冲刷上限（RX 优先的平衡要重新论证） |
| 板端出现卡死或写路径停滞 | 固件不接受多帧写，或 token/缓冲记账有误 | 立刻回退到 K=1 并保留日志定位 |

## 周期 P6：聚合写的帧流布局与 token 归还（修复 P5 审查结论）

### 改动

分支 `sg2002/wifi-opt`，提交 `a8d6fca2e`。审查结论与逐条依据见
`probev5-aggregation-review-20260925.md`（含 P5 四处推理的更正）。

| 位置 | 改动 |
| --- | --- |
| `protocol.rs` | 新增 `stream_frame_len`：一帧在写内占用的字节数 = `align_up(4 + 声明长度, 4)`，即固件遍历的步长 |
| `device/owner.rs` | `ActiveTx` 记录帧流长度 `stream_len`；`append_frame` 追加帧时先丢掉上一帧之后的补齐；`wire_bytes` 只在整笔末尾补 512 |
| `device/data_plane.rs` | 两个发出点都写帧流；`extend_active_write` 逐帧按步长拼接；只在**用户写在飞**时增长写；新增 `take_active_write_tokens` 统一取回在飞写的 token；`complete_write_tokens` 对空集提前返回 |
| `device/progress.rs` | 批次入队失败时归还本帧与批次内其余 token（设备保持 `Ready`）；`drive_shutdown`、`finish_cancel` 取回在飞写的 token |
| `rdif/owner/output.rs` | 推迟完成的 `pending_tx_tokens` 纳入 `has_pending` / `has_runnable_pending` |

**核心约束**（审查确证）：固件按 `4 + align4(声明长度)` 步进遍历一笔写内的帧流，读到 `packet_len == 0` 终止。
逐帧补齐到 512 会让遍历在第 1 帧后终止，**第 2..K 帧静默丢弃**，而 CMD53 正常完成、K 个 token 全部完成、
K 个信用全部扣减、日志无错误。厂商驱动同样逐帧只做 4 字节对齐（`aicwf_sdio.c:2214-2247`），
整笔才在 `aicwf_sdio_aggr_send` 补 512。

### 测试

- 单测：新增 6 条，均先在改前实现上验证为红（帧流遍历、满事件队列下的完成推迟、内部写不携带用户帧、
  批次部分入队、取消归还在飞 token、适配层推迟队列的活性判定）；既有 142 条不变；
- `cargo fmt`、`cargo xtask clippy --package aic8800`、`cargo xtask test --since dev` 通过；
- 镜像：`sg2002_starryos_wifi_sta_aggr4fix_20260925.img`（基底为 P5 镜像，仅换内核 + STA DTB），
  内核 sha256 `9c9cf4c3ec1ef739701efee595419bc56d8d83342ee23faae488c8dd92434296`，
  镜像 sha256 `dd73c5a7781f46f5cedde33c97cac8be0a663c44b1287b8d009ad1cdc17adde5`
  （该镜像构建于迁移前的树；本分支的驱动代码与之一致，dev 的新提交未计入）。

### 现象

原始日志：`probev6-board-20260925.log`（用例顺序：上行 10 s → 下行 30 s → 上行 30 s → 双向 ×2）。

- **停住消失，上行超过 K=1 基线**：上行 10 s 22.4 MB / **18.7 Mbps**、上行 30 s 69.8 MB / **19.5 Mbps**
  （周期 P4 的 K=1 同用例为 30.1 MB / 8.42 Mbps；周期 P5 为 256 KB 后停住）。写侧读数与聚合一致：
  每个窗口约 900 笔写中 626~839 笔是 4 包写（`10+` 桶），单笔 1086~1141 µs ≈ 275 µs/包。
  即 P5 的「事务变快但载荷不达」变为「事务快且载荷到达」，第 2..K 帧被丢弃的因果链由此闭合。
- **下行也更高**：109 MB / 30.01 s = **30.4 Mbps**（周期 P4 27.3、周期 P5 22.3）。单点样本，
  尚不能与运行间波动区分；机制上可信（板端 ACK 走的正是修复前的聚合写路径）。
  驱动侧读路径未改（读尺寸分布与 K=1 同期一致），因此增益来自 ACK/链路层面而非接收代码。
- **双向用例：首轮停住、复跑正常，与 K=1 日志同形**：首轮 `[RX-S] 0.00 Bytes`（从第 1 秒起）、
  `[TX-S] 640 KBytes`（第 1 秒 640 KB 后归零）、`iperf3: the client has terminated`，
  期间驱动侧每窗口仅 2 笔写、4 次读（空闲）、无错误。周期 P4 日志（K=1、无聚合）中同一序列位置
  出现逐字相同的一轮：`[RX-S] 0.00-29.00 sec 0.00 Bytes` + `[TX-S] 640 KBytes 181 Kbits/sec` +
  client terminated，驱动侧同为每窗口 2 笔写、4 次读。故该现象不是本轮改动引入，
  属此前记录的「上层连接停滞」类（周期 M、周期 1、周期 P4 均有先例）；本轮只出现 1 次，
  周期 P4 出现 3 次。
- 仍待补：PC 侧实收计数（`netstat -s` 或抓包）与「上行为何停在 19.5 Mbps」的进一步拆分。

## 周期 P7：聚合的正式形态（2026-09-27）

### 改动

分支 `sg2002/wifi-opt`，提交 `2e017ced6`。

| 位置 | 改动 |
| --- | --- |
| `device/owner.rs` | 新增 `TxAggregation { packets, bytes }`：一笔发送写的两个上界；`set_tx_aggregation` 由「包数」改为该策略，核心默认单帧 / 6144 字节 |
| `device/data_plane.rs` | 增长一笔写时读策略的字节上界（原先是常量 `MAX_AGGREGATE_BYTES`） |
| `rdif/device/endpoints/device.rs` | `AicRdifOptions` 增加 `tx_aggregation` 字段，默认四帧 / 6144 字节（沿用 P5/P6 的上板取值） |
| `rdif/owner/progress.rs` | 适配层不再持有常量：按构造策略设置设备，拉取批次时取策略的帧数上界 |
| `ax-driver/.../aic8800/fdt.rs` | 解析 `aic,tx-aggregation` 与 `aic,tx-aggregate-bytes`；越界（帧数小于 1，或字节数超过发送环一次能交出的总量）时 probe 显式失败 |
| `docs/design/unified-sdio-aic8800.md` | 补帧流布局与冲刷策略，FDT 参数表加入两个新属性 |

### 测试

- 单测新增 1 条：字节上界提前结束一笔写、未装入的帧留在队列等待下一笔；先在去掉字节上界的
  实现上验证为红（4 帧对期望 3 帧）；
- `cargo fmt`、`cargo xtask clippy`（aic8800 3 项、ax-driver 51 项）、`cargo xtask test --since dev` 通过；
- 未上板：默认取值与周期 P6 相同，上板行为不应变化。

### 结论

- **厂商对照**：厂商 SDIO 驱动的 K 是模块参数 `tx_aggr_counter`，默认 **32**，并被同一次流控读数
  截断（`aicwf_sdio_flow_ctrl_msg`），与本驱动的 `aggregate_limit()` 同构；厂商只设帧数上界，
  聚合缓冲按 K 个最大帧备足（fdrv 的 `MAX_AGGR_TXPKT_LEN` 为 `1536*64`，bsp 的同名常量为 `1536*4`）。
- **两个上界的作用面不同**：满帧（1532 字节）在 6144 字节上界下只能装 4 帧，所以**上行大帧流由字节上界决定**；
  而下行的 ACK 小帧流 4 帧只有约 300 字节，**由帧数上界决定**。周期 P6 的下行增益
  （27.3 → 30.4 Mbps）正落在后者上，说明这一侧还有余量。
- 字节上界在追加前判读，属软上界（最多多出一帧）；要精确的每笔帧数时以帧数上界为准，
  字节上界按 帧数 × 1536 放宽即可。
- **credit 单位已核实（本轮顺带）**：信用是固件**缓冲个数**，数据路径一个包扣一个缓冲，与本驱动一致。
  厂商同样以 `aggr_count == fw_avail_bufcnt - DATA_FLOW_CTRL_THRESH` 停止聚合，写完成后
  `fw_avail_bufcnt -= aggr_count`，且 `DATA_FLOW_CTRL_THRESH = 2` 与本驱动的 `RESERVED_CREDITS` 同值；
  厂商命令路径另有 `len > buffer_cnt * BUFFER_SIZE`（`BUFFER_SIZE = 1536`）的字节检查，
  那是命令路径按字节折算的保守限制，不改变数据面的按包记账。原先「credit 单位待核实」一项由此撤下。
- 冲刷策略：写在 owner 需要发送时按此刻已排队的帧成形，在流控读数返回时再增长一次，随即提交，
  不为人造延迟等待更多帧（总线是稀缺资源；周期 P3 显示 88% 的写完成时刻环里已有帧）。

---

## 周期 P8：接收扫描的有界推迟（2026-09-27，已板测：否定，默认已关闭）

提交 `4b4987b27`。

### 改动

分支 `sg2002/wifi-opt`。

| 位置 | 改动 |
| --- | --- |
| `device/data_plane.rs` | `request_receive_scan(now)` 增加准入：有数据帧待发（队首帧 ≥ 128 字节）且连续 ≥ 4 次单块读时，先把 CARD_INT 事实攒住；`RX_DEFER_WINDOW = 1 ms` 到期、或待发帧消失时由 `expire_receive_deferral` 武装扫描 |
| `device/data_plane.rs` | `consume_receive_data` 维护 `rx_small_reads`（≤512 B 记一次，否则清零） |
| `device/owner.rs` | `DataPlaneState` 增加 `rx_deferred_since` / `rx_small_reads` |
| `tx.rs` | `head_frame_len()`：判断队首是数据帧还是 14 字节的 ACK |
| `device/probe.rs` | 新增 `scans`（武装的扫描数）、`deferred`（被攒住的 CARD_INT 次数）、`tx_ready`（武装时已有帧待发） |

**取舍**：这条与 `#2299` 的"接收优先"方向相反，因此只在小帧流 + 本方是发送方时生效；
对端在灌数据（多块读）时立即扫描，不加延迟。

### 测试

- 单测 2 条：① 有数据帧待发时事实被攒住、窗口到期后武装、期间写先发；② 队首是 ACK、
  或接收流是多块读、或发送队列为空时**不**延迟。两条都先在"不做推迟"的实现上验证为红；
- `cargo fmt`、`cargo xtask clippy --package aic8800`、`cargo xtask test --since dev` 通过；
- 镜像：`sg2002_starryos_wifi_sta_rxdefer_20260927.img`（基底为 `…_aggr4fix_20260925.img`，
  只换内核 + STA DTB），内核 `starryos.bin` sha256
  `84a8438e140f780c3698d2cfcb4da5ef05bb34897bc2fe77a29cf92815516c9f`，
  镜像 sha256 `1192fdb387e494671d622591220ac55cc74bc4262edd95e1c89d9e6a0a87ee93`；
  自检：FIT 内 kernel crc32 `c3e680c8` 与 `starryos.bin` 一致、FDT sha256 与
  `www/sg2002/wifi-sta/licheerv-nano-sg2002-sta.dtb`（`d5923a85…`）一致、
  默认配置 `config-sg2002_licheervnano_sd`、Load `0x80200000`、rootfs `/starryos.uimg` 已同步；
- **未上板**：推迟比例与事务条数下降要看板测。

### 现象（2026-09-27 板测，日志 `probev7-board-20260927.log`）

用例顺序（PC 侧 `iperf3 -c 192.168.137.142`）：下行 ×2 → 上行 ×1 → 双向 ×1，各 30 s。

| 用例 | 本轮 | 周期 P6 同用例 | 备注 |
| --- | --- | --- | --- |
| 下行 #1 / #2 | **22.0 / 25.2 Mbps** | 30.4 | 本轮这一次下行**被攒 = 0**（策略未生效），与改动无关 |
| 上行 | **17.2 Mbps**（61.5 MB / 30 s） | 19.5 | 被攒 12.8%（1924/15054） |
| 双向 | 板端 **TX 11.3 / RX 10.7 Mbps**，30 s 正常 | TX 12.5 / RX 6.5 | 未停住；该窗口被攒 148/397 = 37% |

**机制上生效**（上行 28 s 聚合）：`frames/read` 1.77 → **1.97**，事务/帧 1.54 → **1.34**（−13%），
控制读 3735 → **2791**/2s，`owner_calls` 11128 → **9954**/2s，`irq` 11725 → 10170/2s；
下行两轮 `deferred=0`（准入正确地把 ACK 流排除），双向 30 s 无停住。

**但吞吐无法归因**：本轮下行（策略未生效的那一轮）本身比周期 P6 低 17~28%，
同轮两次下行相差 15%（22.0 对 25.2）——说明这次会话整体偏慢（信道/PC 侧），
上行的 19.5 → 17.2 落在这个会话噪声里，**不能据此判定推迟有害或有益**。

### 结论（本轮唯一定量结论：现窗口几乎没有余量）

1. **上行 ACK 是滴流到达的**：28 s 收 47246 帧 ≈ 1687 帧/s，**一帧约 590 µs**。
   1 ms 窗口最多攒 1~2 帧，而基线本来就已经是 1.77 帧/读——**现窗口的可合并空间已经用完**。
2. **要拿收益必须拉长窗口**：按同一份数据折算，3 ms 窗口可把数据读 1716 → 约 675/2s、
   扫描 1075 → 约 360/2s，省下约 260 ms ≈ **13% 墙钟**（数据读 148 µs + 每次读必付的
   状态读 62 µs + 每次扫描收尾的空状态读 62 µs）。代价是每个 ACK 最多晚 3 ms。
3. **实现有一处设计缺陷**（本轮数据暴露）：`bulk_frame_queued()` 只看核心发送队列，
   而一笔 4 帧写成形后队列就被取空、帧在 `active_tx` 里——所以**写在飞时到达的 CARD_INT
   全部判为"不是在当发送方"**，这正是上行只生效 12.8%、双向却有 37% 的原因。
   应把"在飞写携带数据帧"也算作发送方证据。
4. 本轮成本模型修正见 `aic8800-bottleneck-analysis.md` §2.2/§2.3：总线**数据相只占约 11%**，
   开销几乎全在"事务条数 × 每笔往返"上。

### 后续（周期 P8b：修缺陷 + 旋钮 + 交接归因）

针对 P8 结论 3/4 落地三件事（提交 `d28876f67`、`751555277`）：

1. **修缺陷**：`bulk_sender()` 现在把"在飞写携带数据帧"也算作发送方证据
   （`ActiveTx.bulk` 由写入形时的流长度判定），所以写在飞时到达的 CARD_INT 不再被误判；
2. **加旋钮 `aic,rx-defer-ms`**（0~10 ms，FDT，默认 1 ms）：**同一镜像内换臂**
   （0 = 关、3 = 长窗口），不必重编内核；
3. **交接归因**：`[netprobe]` 新增 `park xfer=…us handoff=…us n=…`——把每次 park 按
   "结束它的设备中断"切成两段：中断前（还在跑的传输）与中断后（中断 → owner 重新运行的软件交接）。
   这正是"每笔事务往返是总线时间还是软件交接"的答案。

单测新增 3 条（在飞写算发送方 / 零窗口立即服务 / 既有准入不变），均先在改前实现上验证为红。

**下一轮（配对 A/B）**：同一镜像换 DTB 跑 `aic,rx-defer-ms = 0` 与 `= 3` 两臂，
每臂上行 30 s ×2，各加一轮下行与双向。判据：
① 两臂的上行吞吐配对差（同会话内可比，避开跨会话漂移）；
② `[netprobe]` 的 `park handoff` 占 park 的比例（决定下一轮该往哪走）；
③ 下行与双向是否退化。
若 3 ms 臂不赢或下行退化 → 把推迟整体退回（保留探针计数）。

### 上板操作（配对 A/B）

两臂只差一个 DTB 属性，**内核不变**：

| 臂 | DTB | 属性 | 资产 |
| --- | --- | --- | --- |
| A（关） | `lcn-sta-defer0.dtb` | `aic,rx-defer-ms = <0x00>` | `www/sg2002/wifi-sta/lcn-sta-defer0.dtb` |
| B（3 ms） | `lcn-sta-defer3.dtb` | `aic,rx-defer-ms = <0x03>` | `www/sg2002/wifi-sta/lcn-sta-defer3.dtb` |
| （当时默认） | `licheerv-nano-sg2002-sta.dtb` | 当时版本无该属性 → 1 ms | 第一阶段历史资产；第二阶段代码默认已改为 0 |

本轮套件（内核 `starryos.bin` sha256 `cb46abcd49ff17bbbaf47313b4abca472248c3b00b5bac7abbf3cfca0096a87e`、
crc32 `040bf034`）：

- **镜像 A**（0 ms 臂）：`sg2002_starryos_wifi_sta_ab0_20260927.img`，
  sha256 `d1e0d2cd17332d084b11f04641ddbd5c61e37bbd4de655f78990750e23cc0adf`；
- **镜像 B**（3 ms 臂）：`sg2002_starryos_wifi_sta_ab3_20260927.img`，
  sha256 `f9d7f69fdd227978bb681b940b07424593876a94ae42c26ed3443eec1e30c01d`；
- 两个 FIT 自检：kernel crc32 `040bf034`、默认配置 `config-sg2002_licheervnano_sd`、
  Load `0x80200000`；FDT sha256 分别 `2c5e43a7…`（defer0）与 `bf205624…`（defer3）。

换臂＝**整盘刷另一张镜像**（本机惯用整盘写入，不做分区内替换）：
A 臂刷镜像 A、B 臂刷镜像 B，两张镜像除 DTB 里那一个属性外完全相同。

每臂跑：上行 30 s ×2 → 下行 30 s → 双向 30 s；建议 A → B → A 交替以抵消会话漂移。

### 现象（2026-09-27 配对 A/B，日志 `abA-board-20260927.log` / `abB-board-20260927.log`）

| 用例 | A 臂（`rx-defer-ms=0`，关） | B 臂（`=3`） | P6 对照 |
| --- | --- | --- | --- |
| 下行 | **25.9 Mbps**（92.8 MB/30 s） | **6.27 Mbps**（22.5 MB/30 s） | 30.4 |
| 上行 | **17.2 Mbps**（61.6 MB/30 s） | **1.57 Mbps**（2.25 MB/12 s，未跑完） | 19.5 |
| 双向 | RX 10.7 / TX 12.1 Mbps，30 s | RX 0.21 / TX 0.77 Mbps，15 s | 6.5 / 12.5 |

**B 臂上行被打垮，机制在固件侧**：探针里 park 的**传输段**从 A 臂的 891 µs 涨到 **9176 µs**（10 倍），
而**软件交接段几乎不变**（176 → 117 µs）；写与写之间的空档（有 RX）从 22.8 ms 涨到 **100 ms**。
即"收得不及时"不是让我们晚 3 ms 看到 ACK，而是**让固件整体变慢**（连写完成都要等百毫秒）。
→ **接收保持这条路在固件层面不成立**，与窗口取多大无关。

**A 臂顺带把 P8 的疑问结清了**：A 臂（保持关）上行 17.2 Mbps 与 P8 那轮（1 ms 保持、12.8% 生效）
**完全相同**，而 RX 效率反而更好（frames/read **2.41** 对 1.97、事务/帧 **1.05** 对 1.34、
有 RX 空档 3087 µs 对 3301 µs）。即：**1 ms 保持不但没有收益，还略微劣化了合并效率**；
P6 的 19.5 → 17.2 是会话/环境差异，与本改动无关（现已有干净对照）。

**一处待澄清**：B 臂下行从 25.9 掉到 6.27，但该臂下行窗口 `deferred=0`、`被攒=0`
——**准入根本没生效**，所以这段落差不能用本改动解释，更像会话/环境异常（同轮波动以往只有 ±15~30%，
6.27 超出这个范围）。若要定论，需要重跑一次 B 臂。

### 续（P8c）：P6 回归排查的三臂设计（2026-09-27）

**现象**：上行 P6 19.5 vs 现在 17.2（−12%），下行 30.4 vs 25.9~30.4；用户确认**硬件（板/卡/天线/PC/热点）未变**。

**已排除**：

| 候选 | 判定 | 依据 |
| --- | --- | --- |
| 驱动代码（P7/P8） | 行为等价 | 逐行 diff：P7 是同值替换；P8 的保持只在 `rx-defer-ms > 0` 生效，而**保持 1 ms（P8 轮）与 0（A 臂）上行都是 17.2** |
| 内核基座 `9a7b868ba..714accd8f` | 无可行动机 | 5 个提交里唯一碰调度的是 `8733618c3`，改的是 `sched_get_priority_min/max` 的**上报值**（syscall ABI），不动内核调度；其余为 x86 APIC / virtio-blk / LTP 清单 / card0-vblank |
| 丢包 | 不成立 | 两轮 iperf3 发送端汇总 **Retr 均为 0** |

**仍未排除**（都不是"换了设备"，而是随时间漂移）：信道与协商速率（PC 热点每次开机可能落在不同信道；
本驱动**不记录信道/MCS/带宽**，日志无法证明）、PC 侧状态、板子热状态。
旁证：每笔写耗时 1112 → 1188 µs（+7%）——它测的是 SDIO 事务，但**固件写完成会被空口回压**，
所以这个 +7% 不能当"驱动变慢"的证据。

**三臂**（同一会话交替整盘刷，全部现成、不必重编）：

| 臂 | 镜像 | 组成 |
| --- | --- | --- |
| P6 | `sg2002_starryos_wifi_sta_aggr4fix_20260925.img` | 旧 dev `9a7b868ba` + P6 驱动 |
| 现在 | `sg2002_starryos_wifi_sta_ab0_20260927.img` | 新 dev `714accd8f` + 当前驱动（保持关） |
| **P6@新 dev** | `sg2002_starryos_wifi_sta_p6newdev_20260927.img` | 新 dev `714accd8f` + **P6 驱动**（`a8d6fca2e` 的驱动文件）+ 与"现在"臂**同一张 DTB**（`lcn-sta-defer0.dtb`；P6 驱动不认识该属性，无副作用）→ 两臂只差内核里的驱动代码 |

判读：P6 快而 P6@新 dev 慢 → **基座**；三张一致 → **环境**；P6@新 dev 与 P6 一致而"现在"慢 → **驱动代码**（再二分 P7/P8）。

### 现象（P8d）：P6 复跑——同一张镜像波动 3.4 倍（日志 `p6rerun-board-20260927.log`）

| 用例 | P6 当年（09-25） | **P6 复跑（09-27 傍晚）** | 现在/A 臂（09-27 上午） |
| --- | --- | --- | --- |
| 下行 | 30.4 | **23.8 / 27.1** | 25.9 |
| 上行 | 19.5 | **5.69** | 17.2 |
| 双向 | TX 12.5 / RX 6.5 | TX 9.56 / RX 7.44 | TX 12.1 / RX 10.7 |

**同一张 P6 镜像，上行从 19.5 掉到 5.69（3.4 倍），下行仍在 23.8~27.1。**
上行全程都在 4~10 Mbps（不是中途停住），而同一会话的下行正常。

**三条结论**

1. **"P6 → 现在 −12%" 这条线结束**：单次运行的噪声远大于 12%（同一镜像自己就飘 3.4 倍）；
   而且今天"现在"臂（17.2）实测**高于** P6 臂（5.69）——没有任何证据说明现在比 P6 差。
2. **不稳定的方向就是上行（板子自己发）**：下行跨会话一直稳在 22~30.4，上行在 5.7~19.5 之间飘；
   本次 P6 复跑更是"下行正常、上行垮"。这与"我方上报的空口能力约束了板子自己的发"这条假设同向。
3. **窗口读数说明不是驱动自己的代码变慢**：慢会话里每笔写 1071 µs（历来 1036~1106，无异常）、
   供给 `none=1`（一直有帧可发）、无 RX 空档 130 µs（历来 127~132）——只是每窗口塞进的写更少。
   即变慢发生在驱动自身工作之外（空口/速率方向），不是驱动代码。

**缺的仪表**：`tx_cfm_tag.status` 只带 `tx_done/retry/sw_retry/acknowledged`，**没有速率/MCS 字段**
（`hal_desc.h`）——我们看不到板子实际用什么档发。要看到得走固件的统计路径（新增一条 mailbox 请求 + 解析），
属中等工作量，但它是唯一能判定"是不是速率档位在飘"的手段。

### 阶段收尾（P8e）：OCR 审查一轮的结论与口径更正（2026-09-27）

审查全文存档：`ocr-review-20260927.md`（四名 reviewer，`NEEDS DISCUSSION`，无 blocker）。

**核验结果（我逐条对过源码）**

| # | 审查条目 | 核验 |
| --- | --- | --- |
| 1 | 普通数据逐帧索要固件确认（`hostid = 0x8000_0001`） | ✅ **证实**。厂商 `rwnx_tx.c:647-671` 只对 management/EAPOL(0x8e88)/WAPI(0xb488) 置 bit31 且带唯一序号；普通数据 `hostid = 0`。本驱动 `protocol.rs:207` 恒置位且 RX 收到确认只丢。→ 新 P0 |
| 2 | 已被板测否定的 RX defer 仍是默认 1 ms | ✅ 与我们的 A/B 一致；**默认应改 0**（未授权，未改） |
| 3 | 探针口径：`pkts` 实为写次数、`rx frames` 含确认/指示/print | ✅ 证实 → §2.2/§2.10 的 frames/read、事务/帧、ACK 滴流换算作废 |
| 4 | SDHCI `log_status` 无条件读整组诊断 MMIO | ✅ 证实（`command.rs:338-390` 先读后判级别）→ 低风险可并行项 |
| 5 | capability 需 typed + 遥测 + 分阶段启用 | ✅ 与 §2.9 一致，另加"不对称不能单独证明因果" |
| 6 | 多级复制 → move-only batch + 可回收 DMA | 方向性建议，审查自己也注明"收益未分段计时，不能承诺 2x" |
| 7 | 文档口径需按新证据更正 | 已执行（见下） |

**本阶段已更正的文档口径**

- 三处"把混合 `rx_frames` 当 TCP ACK"的推导作废：`frames/read`、事务/帧、"ACK 每 590 µs 滴流"；
- "厂商双线程 = SDIO 并行"降级（他们在 host 锁上串行）；
- "capability/流水线化 → 约 40 Mbps"标为**推断而非保证**；
- 保留：K=4 的 2.3 倍实测收益、RX defer 的负面 A/B、各轮事务计时。

**审查给出的下一步优先序（采纳）**

1. **P0**：拆 typed counter → 普通数据 `hostid = 0` 的窄 A/B（EAPOL/management 保留确认，且给唯一 ID + 消费者 + 超时）；
2. 空口能力位：typed `MeCapabilities` + golden layout 测试，按 HT40/SGI → VHT → HE 分阶段，每轮记录**实测** bandwidth/MCS/retry/channel；
3. K/byte sweep（6/12/24/48 KiB）——须在 P0 之后重做基线；
4. RX 尾空读：只在 P0 之后仍有余量时做（且注意它会碰 level IRQ 闭合、D80 `OTHER` ack、DC 双 function 不变量）；
5. 复制/DMA 流水线与 ADMA 缓存：主线稳定后再谈。

**待你决定的挂起项**（都不动代码）

- `rx-defer-ms` 默认改 0（或整体退回两个提交）；
- K/byte 默认值的注释修正（审查指出：4 帧/6144 B 是 BSP 的常量，FDRV 默认 K=32、缓冲 `1536*64`）——

### 第二阶段首轮 P0a：普通数据确认请求与探针口径

#### 改动

- `protocol.rs` 将以太网数据帧的确认策略显式化：普通数据经 `TxConfirmation::None` 构造，host descriptor 的 `hostid` 为 0；内部 EAPOL 仍沿用原有确认标记，特殊帧的唯一 ID、消费者与超时仍待单独设计。
- `device/probe.rs` 将 `pkts` 更名为 `tx_writes`，并把接收 FIFO 项拆为 `rx_items`、`rx_data_frames`、`tx_data_confirmations`、`control_confirmations`、`control_indications` 和 `firmware_prints`。
- 统计仍只在 owner 路径更新，不改变解析、完成、credit、取消或 IRQ 顺序；原始日志不改。

#### 验证

- `cargo fmt --all`
- `cargo xtask clippy --package aic8800`：base、`rdif`、`host-test` 三组通过。
- `cargo test -p aic8800 --features host-test,rdif`：152 个单元测试与 1 个公开 API 测试通过。
- `cargo xtask test --since dev`：14 个受影响标准库测试包通过。
- 协议单测确认普通 D80/DC 数据帧 `hostid = 0`，并保留确认策略的显式标记回归；取消、聚合和接收状态机既有测试全部通过。

#### 板测门槛

P0a 实现前的板测门槛原计划使用旧行为/新行为 A/B/A；现按少烧录策略调整为先运行第二阶段镜像一次，记录 TCP TX、TCP RX、双向、重传、credit、RX/TX transaction、IRQ、scan、`tx_data_confirmations` 和控制确认结果。没有同会话旧行为对照时仅记录观察，不作精确收益归因；若仍有 `DataConfirmation`，先按来源区分特殊帧与旧队列，不能直接解释为普通 data confirmation。

### 第二阶段 HT40/SGI：D80 能力 profile 实板

#### 改动

`lmac.rs` 的类型化 `MeConfigProfile` 已实板运行；板端启动日志打印所选 profile 为 `d80-ht40-sgi`，本轮未再改代码。

#### 测试

- 镜像 `sg2002_starryos_wifi_sta_ht40sgi_20260927.img`（SHA-256 `9f17b53862d33e82cf1e68c3213b82f622fa07d7fc9a6f85633228cfae554cc7`）+ `lcn-sta-defer0.dtb`；
- 两轮板测，PC 端统一为 Windows cmd 直接运行 iperf3（每个用例 `-t 30`）：
  - 第一轮 8 个用例：正向 `-c` / `-b 0` / `-b 100M`，反向 `-c -R` / `-b 0` / `-c -R` / `-b 100M`，末尾 `--bidir`；原始日志 `com6-board-20260928-023449.log`。
  - 第二轮 3 个用例：正向、反向、双向各一轮；原始日志 `com6-board-20260928-024750.log` 与 PC 侧 `pc-iperf3-20260928-024750.log`。

#### 现象

| 用例（PC 端命令） | P0a 轮（`234650`） | HT40/SGI 第一轮 | HT40/SGI 第二轮 |
| --- | --- | --- | --- |
| 正向（PC→板，板接收） | 16.4 / 20.7（30 s） | 25.3 / 28.8 / 26.7 | 29.9 |
| 反向（板→PC，板发送） | 16.0（10 s） | 18.8 / 15.6 / 19.4 / 19.0 | 20.6 |
| 双向（板发送 / 板接收） | 9.69 / 6.14 | 未完成（断链） | 13.6 / 12.5 |
| 反向 `-b 100M` | 6.71（10 s） | 19.0 | 未测 |

- 两轮的正、反、双向读数都高于 P0a 轮同向值，但它们是不同会话；反向用例的时长也不同（P0a 轮为 10 s，本轮为 30 s）。因此按观察记录，不作为 capability 的因果收益。
- 上轮"`-b 100M` 明显偏慢"没有复现：第一轮正向三个用例为 25.3 / 28.8 / 26.7（`-b 100M` 居中），反向四个用例为 18.8 / 15.6 / 19.4 / 19.0（`-b 100M` 与不带 `-b` 持平）。上轮 6.71 的那一次按会话内偶发处理。
- 所有活跃窗口 `data_confirmations=0`，P0a 的普通数据确认语义保持不变；板端与 PC 端在同一用例上读数一致（29.9/29.9、20.6/20.6、13.6/12.5 对 13.6/12.6）。
- 板端发送窗口（第二轮反向用例）的瓶颈读数：`credit` 每 2 s 读数 900~1400 次，`min=2`（保留位）、均值仅 3.3~7.9（该字段以 `avgx10` 打印），回退 500~1300 次/2 s（每次约 0.55 ms）；单笔写均值约 1.0 ms。即固件发送缓冲长期见底，板端发送仍受固件/空口排空速度限制，与 §2.10 的慢会话形态同形。
- 板端发送时 `rx` 与 `tx` 的队列深度读数显示核心队列经常积压（`queue_at_arrival` 的 4+ 桶占多数），说明上游供帧不是限制项。

#### ARP 表项到期事件（板载网络栈，非 AIC 驱动）

两轮板测都在板端发送或双向用例中出现连续数十行 `wlan0: sending pending IPv4 packet to 192.168.137.1 via 66-d6-9a-fb-71-56`。本轮定位为 `net/ax-net` 邻居表的固定 TTL 到期后重新解析、并成批冲刷待发队列：

- `net/ax-net/src/device/ethernet.rs` 的 `NEIGHBOR_TTL = 300 s`、`net/ax-net/src/consts.rs` 的 `ETHERNET_MAX_PENDING_PACKETS = 128`；
- 第一轮：`t=18.589` 学到网关 MAC，`t=318.593` 打印 `requesting ARP for 192.168.137.1`（相距 300.0 s），`t=318.639` 打印 `Pending packets buffer is full, dropping packet`，`t=318.651` 起冲刷 65 个待发包；
- P0a 轮同形：`t=10.385` 学到，`t=310.388` 重请求（相距 300.0 s），`t=310.576` 连续丢包两次，`t=310.595` 起冲刷 65 个；
- 受影响那一个反向用例的逐秒吞吐在事件后单调下滑（19.9 → 18.9 → 12.6 → 10.5 → 9.46 → 8.40 → 8.38 → 8.37 → 7.36 Mbits/sec），同轮其后的反向用例恢复到 19.4 / 19.0，即影响限于事件所在的连接；
- 事件窗口内 `credit` 均值下降、回退次数上升，指向固件/空口排空变慢；机制未证实。

#### 双向用例断链（第一轮第八例）

`--bidir -t 30` 开始后板端只剩零星 512 B 写、接收计数为 0，且未记录任何驱动错误事件；PC 侧观测为 Wi-Fi 断链。第二轮同一命令正常完成（13.6 / 12.5 Mbps）。按偶发记录，暂不立项。

#### 观测缺口

协商带宽/MCS/重传仍不可读。厂商在 `aic8800_fdrv/rwnx_main.c` 的 STA 信息路径用 `MM_GET_STA_INFO_CFM.rate_info` 解出 `bwTx`/`formatModTx`/`mcsIndexTx`/`giAndPreTypeTx` 以及 RSSI 与 ack 成功/失败计数，位域定义在 `hal_desc.h` 的 `union rwnx_rate_ctrl_info`；该请求的 payload 是 4 字节 compat 形式（`sta_idx` 加 tag `sta`；vendor 只对 D80X2 及以上发一字节形式），`MM_GET_STA_INFO_REQ`/`CFM` 为 `0x0075`/`0x0076`，与本驱动已用的 `MM_GET_MAC_ADDR_REQ` 同段枚举。这是补最小遥测的直接入口。

### 第二阶段速率遥测：MM_GET_STA_INFO 最小读数（2026-09-28）

#### 改动

- `lmac.rs`：新增 `MM_GET_STA_INFO_REQ`（`0x0075`）/`MM_GET_STA_INFO_CFM`（`0x0076`）、4 字节 compat 载荷（`sta_idx` + tag `sta`）与 `parse_sta_info()`：把 `union rwnx_rate_ctrl_info` 的位域解成带宽/调制格式/MCS/NSS/短保护间隔与 retry，并带上 RSSI 与 ack 成功/失败计数。MCS 与 NSS 按格式分别解包（HT 用低 3 位、VHT/HE 用低 4 位、legacy 为速率索引）。
- `device/`：关联成功时排定第一次采样（一个周期之后），此后每 1 s 一次。采样走同一个单 mailbox，确认仍以 `WaitForInterruptUntil(deadline)` 等待；空闲时由该期限驱动 owner 醒来取一次样本。
- 该读数只用于板测观测：请求超时或确认长度不符时停止采样并记一行日志，**不判定链路失败**，也不参与发送、credit、接收或取消决策。

#### 验证

- `cargo fmt --all`；`cargo xtask clippy --package aic8800` 三组通过；`cargo xtask test --since dev` 通过（aic8800 162 个单测 + 1 个公开 API 测试）。
- 新增 7 条单测：请求载荷布局、三种格式的速率解码与计数器、错误长度拒绝、关联后采样并重排下一次、不可用确认与请求超时都只停读数、控制命令优先、未关联不采样；关联排定采样的断言并入既有的关联指示用例。

#### 上板判据

- `[wifi-sta-info] width=..MHz format=.. mcs=.. nss=.. sgi=.. retries=.. rssi=..dBm txfailed=.. ackok=.. ackfail=..`
- 关键问题：板端发送时 `width` 是否到 40、`format` 是否 `ht-mf`、`sgi` 是否为 1；速率档是否随吞吐波动而变；ack 失败率是否与 credit 见底同时出现。
- 副作用：每次采样多一次 flow-control 读；一次采样占 mailbox 约一个往返。

### 第二阶段 VHT/HE 能力：厂商对照与 D80 profile 扩展（2026-09-28）

#### 缘起：遥测读数把问题换了位置

遥测复测轮（`com6-board-20260928-042935.log`）的 259 条读数给出两条事实：

- 链路一直在 **20 MHz**，且 **`format=ht-mf`、`mcs` 以 7 与 6 为主、`sgi=1`**——即 HT 能力位（含 SGI）已经生效，固件把发射档位顶在 HT20 的上沿（MCS7+短 GI，PHY 72.2 Mbps）；带宽没有到 40 MHz。
- ack 计数到会话末尾为 362135 成功 / 66444 失败（约 15%；会话早期一度接近 1:1）。该计数在停止发包后冻结，说明它随流量更新；其口径（每次尝试、还是每个聚合）尚未核实，按观察记录。

由此，"HT40 没生效"这句话需要拆成两半：**HT 侧（含 SGI、MCS7）生效了，缺的是带宽**。而带宽在 20 MHz 上已无余量，剩下能动的只有调制格式。

#### 上一轮一处判读的更正（重要）

HT40/SGI 轮曾据厂商 Linux 基线的 UDP 76.5 Mbps 推断"该热点能给到 40 MHz"（理由：超过 20 MHz 单流 HT 的 PHY 上限 72.2 Mbps）。**该推断不成立**：厂商 D80 路径在 2.4 GHz 上同时宣告 VHT 与 HE（`rwnx_main.c` 的 2.4 GHz band 带 `.vht_cap` 与 `iftype_data = &rwnx_he_capa`，`rwnx_mod_params.c` 把 `vht_cap_2G.vht_supported` 置为 true，HE 能力位里还有 `20MHZ_IN_40MHZ_HE_PPDU_IN_2G` 这种 2.4 GHz 专用位）。20 MHz 下的 PHY 上限因此不是 72.2：VHT20 MCS9 为 86.7 Mbps，HE20 MCS11 为 129–143 Mbps。厂商基线的 58.7/76.5 Mbps 完全可以发生在 20 MHz 上。

因此：本链路的 AP 宽度重新回到未知；"我们卡在 20 MHz 是自己配置的问题"这一说法不成立（也可能只是 AP 只给 20 MHz）。真正确定的差距是**调制格式**：vendor 是 HT+VHT+HE 客户端，我们是 HT-only 客户端。

#### 厂商 `me_config_req` 逐字节对照（只读）

对照 `aic8800_fdrv` 的 `lmac_msg.h`、`lmac_mac.h`、`rwnx_msg_tx.c`、`rwnx_mod_params.c`、`rwnx_main.c` 与 `drivers/net/aic8800/src/lmac.rs`，先排除了一批假设：

| 对照项 | 结论 |
| --- | --- |
| `me_config_req` 布局 | HT 32 / VHT 12 / HE 56 / `tx_lft`@100 / `phy_bw_max`@102 / … / `dpsm`@109，总长 112；HT 块内部（capa_info@0、ampdu@2、mcs_rate@3..19、ht_extended@20、tx_beamforming@24、asel@28）一致 → 不存在"字段写进保留字节"的错位 |
| `mac_htcapability` 取值 | A-MPDU 参数 `0x1f`（factor 3 = 64K、density 7 = 16 µs）、`tx_params = HT_MCS_TX_DEFINED`、`rx_highest`（150 = 厂商 `150 × nss`，nss 在 chipid ≤ D80 时被强制为 1）一致 |
| 关联请求 IE | 厂商也只把 `sme->ie`（RSN 类）放进 `ie_buf`，HT/VHT/HE IE 由固件按 `ME_CONFIG` 生成 → 我们没有漏发 HT IE |
| `sm_connect` 信道提示 | 厂商在不知道 BSS 信道时同样填 −1；本驱动不下发 scan，始终走该分支，与厂商该分支一致 |
| 信道表 flags | 该 ABI 的 `mac_chan_flags` 只有 `NO_IR`/`DISABLED`/`RADAR`，**没有 HT40 位** → 信道表不可能门控 40 MHz |
| `ps_on` / `dpsm` / `he_ul_on` | 1 / 0 / 0，与厂商默认一致 |

发现并修正的**唯一取值偏差**是 `phy_bw_max`：厂商对 D80/D80X2 **强制** `use_80 = true`（`rwnx_msg_tx.c`），因此 D80 发送的是 `PHY_CHNL_BW_80`（2）；DC 的 RF 只到 40 MHz，厂商按固件上报的 RF 带宽得出 `PHY_CHNL_BW_40`（1）。本驱动此前把两者写反了（Conservative=2、D80=1）。本轮把 D80 改为 2；Conservative 保持 2 未动（DC 不在这条测试线上，且本驱动不读 RF feature 寄存器，见 `lmac.rs` 中该函数的注释）。

厂商 D80 在 2.4 GHz、单流下实际发送的能力取值（本轮照此编码）：

| 块 | 取值 |
| --- | --- |
| HT | `0x0063`（LDPC、20/40 MHz、SGI20/40）、rx_mask `[0xff,0,0,0,1,…]`（MCS32）、`rx_highest=150` |
| VHT | `vht_capa_info = 0x03987111`、rx/tx MCS map `0xfffe`（单流 MCS 0–9）、rx/tx highest `390` |
| HE | `mac_cap_info[2]=0x02`、`phy_cap_info=[0x06,0xe0,0x2b,0x58,0x0d,0xc0,0xcf,0,0x02,0x30,0]`、rx/tx `mcs_80 = 0xfffe`（单流 MCS 0–11）、160/80+80 全不支持、PPE 阈值 `[0x38,0x1c,0xc7,0x01,…]` |
| 标量 | `phy_bw_max = 2`、`ht_supp = 1`、`vht_supp = 1`、`he_supp = 1`、`he_ul_on = 0` |

厂商的两处差异本轮**有意不跟**：HT 块的 RX_STBC 与 MAX_AMSDU 位（都是接收侧能力，涉及 A-MSDU 接收路径，需单独论证）；`ant_div_on` 厂商默认开启，本驱动仍为 0。

#### 改动与验证

- `lmac.rs`：`MeConfigProfile` 的 D80 分支改名为 `D80Ht40SgiVhtHe`（启动日志随之打印 `d80-ht40-sgi-vht-he`），新增类型化的 `VhtCapabilities` / `HeCapabilities` 与各自的 `encode_into`，`me_config_payload()` 按 profile 写入 VHT/HE 块并把 `vht_supp`/`he_supp` 置为该块是否存在；DC 分支（`Conservative`）字节不变。
- 提交 `1fb1bc91c`。验证：`cargo fmt --all`、`cargo xtask clippy --package aic8800`（base / `rdif` / `host-test` 三组）、`cargo xtask test --since dev`（aic8800 通过）。
- 新增金样本单测固定 VHT/HE 块的每个字节与 MCS map、PPE 阈值、结构内偏移（含 HE 结构的对齐填充字节）与 `vht_supp`/`he_supp`；`startup` 路径的两芯片对照表也加了 VHT/HE 支持位断言。

#### 上板判据

镜像 `sg2002_starryos_wifi_sta_vhthe_20260928.img`。判据集中在遥测的 `format`/`mcs` 字段：

- 若读数出现 `format=vht` 或 `format=he-su` 且 MCS 进入 8–11 → 差距来源是调制格式，厂商基线的领先可归因；
- 若仍是 `format=ht-mf mcs=7` → 说明该 AP 不与我们协商 VHT/HE（或固件未启用），下一步转向 AP 侧与带宽问题，而不是继续加客户端能力位；
- 只要 `width=20MHz` 保持，就不要把吞吐差异解释成带宽收益。

回滚：`sg2002_starryos_wifi_sta_stainfo_20260928.img`（等价于 `8a350076a`）与 `sg2002_starryos_wifi_sta_phase2_p0a_20260927.img` 都在本地构建产物目录。

#### 实板读数与热点状态变量（2026-09-28）

**VHT/HE 生效**。第二遍（`com6-board-20260928-050910.log`）204 条读数全部 `width=20MHz format=he-su`，
MCS 分布以 9~11 为主（11:60、10:38、9:29、8:21、7:22、6:14、5:8、4:5），即板端在 **HE20 的顶档**
（MCS11 = 1024-QAM，PHY 约 143 Mbps）发送，上一轮认定的"调制格式差距"已经补上。该遍吞吐：接收 30.6、
发送 26.9、双向 板发 20.8 / 板收 8.76 Mbps。

**同一镜像第一遍极低**（`com6-board-20260928-050653.log`）：接收 15.7、发送 2.50（13 s 中断）、双向 1.05 / 1.97 Mbps。
该遍的读数形态与第二遍完全不同：MCS 在 `he-su 0~5` 与 `format=non-ht mcs=3`（legacy 档）之间跳动，
ack 失败 12266 对成功 27677（约 31%），且 `rssi` 为 −31~−41 dBm，而第二遍为 −18~−26 dBm。
两遍的驱动启动日志（profile 行、关联、EAPOL、tunnel）结构一致，无错误行，主机侧发送功率表是固定常量。

**两遍之间的变量不止一个**：第二遍是**板卡拔电重启 + PC 热点重开**之后测的，因此"重启热点就好了"这个
结论尚未成立。当前有两个候选成因，都能解释观测：

| 候选 | 能解释的观测 | 不能解释的 |
| --- | --- | --- |
| PC 热点侧状态（信道漂移/单射频与 PC 自身 station 连接分时/适配器省电/ICS 状态） | 双向都变慢、ack 失败升高、RC 掉档 | 本板收到的 `rssi` 下降 13~15 dB（除非 AP 发射功率/信道配置本身变了） |
| 板卡侧 RF 状态（固件校准或增益/功率索引在本次上电后不正常） | `rssi` 下降（本板接收增益低）、ack 失败升高（对端听不清本板）、两个方向都慢 | —（单一原因即可解释全部） |

第三种可能（本驱动的收发路径把 AP 拖坏）证据较弱：下行方向（PC→板，完全由 AP 决定发射）同样腰斩，
而固件自己的速率控制掉到 legacy 档是"空中链路变差"的结果，不是主机软件能直接造成的。

**待做的判别实验（都便宜）**：

1. **分离两个变量**：只重启热点（板卡保持运行）→ 若恢复，指向热点；只拔电重启板卡（热点不动）→ 若恢复，指向板卡 RF 状态。
2. **退化随时间的形状**：热点刚重开后不做任何改动，每 5 分钟跑一轮 60 s 参考测量，持续 1 小时。
   若单调下滑 → 与"在线时长"相关，且短间隔复跑会掩盖它（会导致把退化误归因到本轮改动）；
   若在某点阶跃 → 是事件（信道切换、roam、适配器省电等），可对照 PC 侧日志定位。
3. **厂商镜像对照**：厂商 Linux 镜像用同一两遍协议跑一次；厂商也塌则与 StarryOS 驱动无关。
4. **PC 侧读数（零成本）**：测试期间在 PC 上记录热点信道、`netsh wlan show interfaces` 的 station 状态与
   信道、以及 PC 自身是否有上行流量（单射频分时是首要怀疑对象）。

**对已有结论的影响**：此前各轮之间只有"跨会话观察"的吞吐对比（P0a 16.4/20.7、HT40/SGI 25.3~29.9、
遥测轮 30.5），现在知道会话内的第一变量可能不是我们改的东西，这些对比不能作为收益证据；
后续测量按下面的协议执行，并只在"同一热点会话内、且读数健康"的样本之间比较。

#### 板测协议（2026-09-28 起）

**两个敏感操作**：重启 PC 热点、重启板卡。这两者在同一次测量内必须受控——测量期间都不做，或在测量前统一执行并记录；
禁止把其中一个当作"修好了"的手段而不记录（2026-09-28 的两遍就是两者同时发生，导致成因未分离）。

1. 每次测量前**重启热点**并等待约 1 分钟，使所有轮次都落在"热点刚开"的同一状态；
2. 每轮先跑一遍标准三向（60 s 各方向）作为**参考臂**，只有参考臂落在历史正常区间才采纳该轮的对比数据；
3. 记录本轮开始时间、热点在线时长，以及板卡自本次上电起的运行时长；
4. 用遥测读数作为**会话健康记录**：`format`/`mcs` 分布与 ack 计数随轮次归档，出现 legacy 档或 MCS 大面积塌陷时标注该轮数据不可用于归因。

### 判据（上板后读探针行）

0. **先看准入门是否稳定**：trickle 门（连续 ≥4 次单块读）会被**成功合并后的读**重置——
   但上行一次窗口只攒 2~3 个 ACK（约 150~220 字节），仍在一个 512 B 块内，所以计数会继续递增、
   推迟得以持续；只有当一次读真的跨到 2 块（>6 个 ACK）时门才会重置，此时下一批事实立即服务、
   随后重新累积。**这是自限的、偏保守的行为**；若上板看到 `deferred/scans` 忽高忽低，
   先确认是不是这个重置在起作用，再谈调窗口。
1. **`deferred` 占 `scans` 的比例**——策略实际生效的比例；
2. **每窗口 RX 事务条数**（`data n` + `control n`）与 `frames/read`——合并是否发生；
3. 上行吞吐与下行不退化；双向用例不新增停住。

---

## 第五阶段第二轮：以「让 SDIO 尽可能忙碌」为目标重构（2026-09-29，进行中）

### 改动（S0：补仪器，无行为变更）

**目标口径已变更**：不再以吞吐是否显著提高为收益指标，改以「异步架构使 SDIO 尽可能忙碌」为达成目标。
验收分三层——A 架构层为「总线槽为空且任一单元有 ready offer」的驱动侧空档，且**必须按原因分类**；
B 机制层为 `completion_to_commit`（出分布不出均值）；C 结果层为总线腿占比，**仅观测、不作验收**。
方案与实施阶梯见执行方案 §11。

S0 只加计数器，不改任何决策：

- `owner_flush_blocked`（`device/probe.rs` 全局静态，调用点在 `rdif/owner/progress.rs:161` 的
  `outputs.flush()` 提前返回处）：发布环满、整步不推进的次数。此前这个状态在报告里完全不可见。
- `note_tx_harvest_at`/`book_tx_harvest_gap`：量「总线从空闲到再次被占用」的整段墙钟
  （`[wifi-probe-time]` 的 `harvest_to_bus=…us/… long=…`），供判据 1 把交接段拆成
  「调度恢复」与「驱动代码」两部分。起点取**完成中断进入时刻**（与既有的 `irq_handler`/`post_irq`
  两栏同一锚点，三者相加即交接段全貌），收口在**每一次**事务提交前，而不是只在写之前：
  同一时刻只有一笔事务在飞，若按写收口，完成之后先跑的那笔读会把它的整段往返算进交接空档。
  超过 20 ms 的空档计进 `long` 而不入均值，那是「栈里没有帧」的停摆，不是驱动自己的交接。
- `credit_reserved_backoff`：保留量分支（`credits <= 2`）的重试次数与等待时长单列
  （`[wifi-probe]` 的 `reserve backoff=… avg=…us`）。此前它与薄池分支的 `backoff=` 合并，
  而旋钮 `aic,tx-credit-wait-us` 只作用于后者。
- `note_credit_over_claim`：成形时超出 credit 预算的笔数与包数（`[wifi-probe]` 的 `credit_over n=… pkts=…`）。
  这是 S1 缺陷（`stage_next_transmit` 把 `saturating_sub(in_flight)` 写在 `.max(1)` 之前）
  在下一次上板时的直接证据。
- 供给口径修正：`write_done` 的 `core_ready` 补上 `staged_tx`。成形到后继槽的帧已经离开核心队列，
  只读队列会把「核心手里正握着后继」报成 `supply none`。

- `ax-net` 侧只加两个计数打印（本阶段唯一一次触及该目录，无行为变更）：
  `more_round` 计 `poll` 返回 `More`、该轮不推进 owner 的次数（`[netprobe]` 的 `more=`），
  `rearm_race` 计重武装时发现已有工作发布的次数，即「若无该复查会被丢掉的唤醒」（`rearm_race=`）。

### 改动（S1：后继写的成形上限）

`stage_next_transmit` 原先写 `min(credits - 2, policy) - in_flight`，两个方向都错：策略上限管的是
一笔写、不与在飞写共享，把它也减掉会在 credit 充足时把后继压到 `policy - in_flight`；
而归零后的 `.max(1)` 又会在 credit 紧张时成形一帧，把在飞写将要花掉的固件缓冲再认领一次。
改为 `successor_limit(in_flight) = min(credits - 2 - in_flight, policy)`，未知读数取 0，
且预算为零时不调用 `form_transmit`（它无论如何会先取一帧，传零拦不住）。
`credit_over` 因此恒为 0，由缺陷证据转为回归守卫。

### 测试

`cargo fmt --all` 干净；`cargo xtask clippy --since dev`（7 包 186 项）与 `cargo xtask test --since dev` 全过；
`cargo test -p aic8800 --features host-test` 163 + 1 全过。
S1 新增两条单测——credit 充足时后继取满策略、读数已被在飞写花完时不成形——
按项目要求先做了变异验证：两条都在改前的实现上必然失败（分别读到 1 帧、以及「仍然成形了一帧」）。

### 结论

S0 无行为变更，可与既有 `ahead1` 镜像直接 A/B 而不必重烧对照臂。S1 改变成形上限，
其效果是 **C 类读数**（写长度分布），不许当作 A 层的收益记。
三个待答判据（执行方案 §11.4）里，判据 2 由既有的分类往返与 `legs`/`stages` 两栏回答，未新增计数器；
判据 1 与判据 3 由上列计数器在下一轮上板时回答。

### 本轮要构建的镜像

`sg2002_starryos_wifi_sta_instr_20260930.img`：基座 `sg2002_starryos_wifi_sta_ahead1_20260929.img`，
只换内核（工作树 `a6633f3f6` + 未提交的 S0/S1 改动），DTB 沿用 `lcn-sta-ahead1.dtb`
（S0/S1 未引入新 FDT 属性；该 DTB 已与 `lcn-sta-ahead1.dts` 逐字节比对一致，且带 `/chosen/rng-seed`）。
不给 `--overwrite`，`-o` 直接指向构建产物目录。SHA-256 `5292431d…`（见镜像表）。

组装后的独立复核（脚本自检之外）：从镜像 p1 取出 `boot.sd`，`dumpimage` 拆出的 kernel 与
`target/riscv64gc-unknown-none-elf/release/starryos.bin` 逐字节相同、fdt 与 `lcn-sta-ahead1.dtb`
逐字节相同；p2 的 `/starryos.uimg` 与同目录 `starryos.uimg` 相同；FIT 三镜像与默认配置
与上一轮 `ahead1` **完全一致**（同带 `ramdisk-1`）；内核里逐一确认 S0/S1 的格式串齐备
（`flush_blocked=`、`credit_over n=`、`harvest_to_bus=`、`reserve backoff=`、`more=` 与 `rearm_race=`）。
基座镜像时间戳未变，未被就地改写。

### 判据（上板后读探针行）

按执行方案 §11.4 与 §11.6，本轮读数要回答两个判据、并给三处机制计数与非退化做守卫：

| # | 读什么 | 在哪一行 | 判读 |
| --- | --- | --- | --- |
| 1 | **`More` 路径的真实贡献**（判据 3） | `[netprobe]` 的 `more=` 与同行 `polls=`/`owner_calls=` | 若 `more` 远小于 `polls`，判据 3 的答案就是「贡献≈0」，**S8 直接删除**，不必动 `ax-net` |
| 2 | **交接空档归因**（判据 1） | `[wifi-probe-time]` 的 `harvest_to_bus=…us/… long=…`，与同行的 `irq split pre/isr/post` | `harvest_to_bus` 已经把「调度恢复」算进去；若它接近 `post` 而远大于 `legs/program`，这段不是驱动代码，S6 的「合并两次 owner 调用」不在钱上 |
| 3 | **发布环是否真的会满** | `[wifi-probe]` 的 `flush_blocked=` | 若恒 0，S7 的前提不成立；若非 0，它就是被 `outputs.flush()` 挡住的那部分墙钟 |
| 4 | **credit 重试的构成** | `[wifi-probe]` 的 `backoff=`（薄池 3..=7）与 `reserve backoff=`（保留量 ≤2） | 保留量分支应占约 78%；若否，说明此前从代码推出的构成不对 |
| 5 | **S1 回归守卫** | `[wifi-probe]` 的 `credit_over n=… pkts=…` | **必须为 0**；非 0 说明成形仍在超出 credit 预算 |
| 6 | **S1 的 C 类读数** | `[wifi-probe]` 的 `size blk … 10+ n=…` 与 `write … bytes=` | 与 `ahead1` 四次运行对照；写长度差是否消失属 **C 类观测，不作 A 层验收** |
| 7 | **非退化** | 上行吞吐、TCP RX、双向用例 | 双向不新增停住；吞吐按 §4.3 的协议只在同会话内比较 |

### 现象（2026-09-30 实板，日志 `(2026-09-30_095951)`）

一次启动内四个用例：下行 31.6 Mbps、**上行两遍（先 15.4，重测 32.4）**、双向 RX 11.6 / TX 23.1 Mbps。
下面所有数字都出自这一份日志。

**口径登记（先登记再算数）**：

- 窗口 = `[wifi-probe]` 的 2 s 统计窗（`dt≈2000ms`，共 91 个）。
- 分段按 iperf3 任务退出时刻切，并用各用例 30 s 的名义时长反推起点：上行第一遍 82.7–113.0 s、
  上行重测 122.8–153.1 s、双向 155–192 s。
- `harvest_to_bus` 是窗口内的**和 ÷ 样本数**；样本是「任意事务完成 → 下一笔事务提交」且间隔 ≤20 ms 的
  那些，因此样本数大于 `tx_writes`（读/控制事务也会收口）。**它不按事务类别分开**，这是下面把
  「驱动自己那段」写成相减结果时必须记住的限制。
- 完成中断分项 `pre/isr/post` 只统计 TX 类，按样本数加权。
- 总线腿占比 = 主时间线的 `tx busy` ÷ 窗口时长，**逐窗取值再平均**（不是全段求和后相除）。
  用这个定义复核同日的历史日志：ahead1 41.4%/48.1%、nowait 47.3%——**瓶颈文档里的「36%」出自
  2026-09-28 那次会话，用本定义在同日日志上复现不出来**，故本轮只与同日日志做 like-for-like，
  不与那个 36% 直接比。

**判据 3（`More` 路径的真实贡献）——已有答案**：`more` 全日志合计 **155 / 200473 polls = 0.077%**，
91 个窗口里 76 个为 0，非零的最大 17。`poll` 返回 `More` 而该轮不推进 owner 这条路径**贡献≈0**，
**S8 删除，不必动 `ax-net` 的执行器循环**。

**判据 1（交接空档归因）——已有答案，数字已按 2026-09-30 独立复核更正**：

| 段 | `harvest_to_bus` 均值（样本） | TX 完成中断 `pre` / `isr` / `post` |
| --- | --- | --- |
| 上行第一遍 | 703 µs（8719） | 2077 / 1 / **458** µs |
| 上行重测 | 755 µs（10138） | 2091 / 1 / **412** µs |
| 双向 | 707 µs（8617） | 2074 / 1 / **380** µs |

- 唤醒/调度恢复（`post`）是整段里最大的一项，但**占比必须写口径**：逐窗比值再平均是 45.6–63.9%，
  按样本加权（各自求和再相除）是 **36.0%**——差在分子只算 TX、分母含读写。**以 36% 为准。**
- `post` 的终点**不是「核心取走完成」**，而是「中断处理退出 → 取走该完成的那次 `advance` 拿到的时钟」。
- 「驱动自己那段 = 整段 − `post`」是**跨总体**的减法，不能这么叫；同窗 `legs` 能直接量到的下游成本只有
  下一笔写的 `dma`（125–158 µs）与 `program`（42–64 µs）。
- 替代解释已否证：`post` 不是执行器一轮循环的节拍（`corr(post, P) ≈ 0`），且它与执行器独立测的
  park 交接逐窗几乎相等（比值 0.88–1.00，相关 0.87）——两种独立测法互证这段就是唤醒。
- **判读更正**：原写「`rearm_race` 60.6% 说明执行器频繁被重新调度而不是停在 park 上」**方向是反的**。
  越是从头到尾停在 park 上的空载窗该比值越高（17 个空载窗 `rr/oc = 74.9%`，那些窗
  `waits × wait_us / dt ≈ 0.999`，整窗停在 wait 上）。`rearm_race` 不是「没在 park」的证据。
- **覆盖口径**：`harvest_to_bus` 只在块读/块写两个 DMA 分支收口，`Direct`（含 credit 读与大部分
  mailbox/控制）与 `Bus` 分支不收口，它们之间的空档既不进均值也不进 `long`。

**结论：交接空档里唤醒/调度恢复是最大的一项，S6 的「合并两次 owner 调用」动不到它**——
同窗能被 `legs` 直接量到的下游成本只有 `dma` + `program` 约 0.17–0.22 ms/笔。

**另两条判据前提不成立、可以删**：

- `flush_blocked=0`：91/91 窗为 0，`outputs.flush()` 从未挡住过任何一步 ⇒ **S7 的前提不成立**。
- `long=0`：91/91 窗为 0，本轮没有超过 20 ms 的停摆。

**S1 的回归守卫通过**：`credit_over n=0 pkts=0`，91/91 窗。恒等式也全部闭合：
`chain` 七项之和 = `tx_writes`、`staged ready + missed` = `tx_writes`，均 91/91 窗零违例。

**口径更正（两项）**：

- 保留量分支（`credits ≤ 2`）占 credit 重试 **67.7–70.4%**（三段分别 70.4 / 69.7 / 67.7），
  此前按代码推算写作「约 78%」。方向不变（旋钮够不到的那一支是多数），数值以此处实测为准。
- credit 等待**不可与 `accounted` 相加**：该跨度含重试读自身的往返（该窗 ≥23 ms、约 1.2 点），
  且退避期间 RX 不被挡住。段内饱和窗（n=33）中位 15.9%、均值 18.3%、分布 7.0–45.8%，不是常数。
  credit 等待与 deadline park **是同一段**（credit 分支设 `retry_at` → 成为 park 的 deadline）。

**上行两遍的对照（同一次启动内，这是本轮最有信息量的一条）**：

| 项 | 第一遍（退化） | 重测 |
| --- | ---: | ---: |
| 驱动交给 SDIO 写的线上字节 / 30.3 s | 119.6 MB（31.6 Mbps） | 135.4 MB（35.7 Mbps） |
| iperf3 交付 | 55.0 MB（15.4 Mbps） | 116.0 MB（32.4 Mbps） |
| **交付 / 线上** | **0.49** | **0.91** |
| 每笔写 | 20.5 KB | 20.4 KB |
| 写周期 | 5861 µs | 4941 µs |
| 保留量 backoff 占比 | 70.2% | 69.7% |
| `staged ready` 率 | 51.3% | 57.7% |
| 总线腿占比 | 44.7% | 50.1% |

**驱动侧行为两遍几乎一致（每笔写只差 0.5%、保留量占比只差 0.5 点），而交付差 2.1 倍。**
退化的变量在 SDIO 之下——空口/链路侧，不是本轮改动。这与既有的「同一镜像两次开机可差 24%」
同源，但更强：这次是**同一次启动内背靠背的两遍**。`[wifi-sta-info]` 上两段的 `mcs`（7–11）、
`rssi`（−19 ~ −23 dBm）也都不支持「退化段链路更差」的简单解释，具体成因未定位，
按既有约定不在驱动侧追。

**S1 的 C 类足迹（不作 A 层收益）**：`staged ready` 率由 `ahead1` 的 85–90% 降到 51–58%——
credit 吃紧时后继不再成形（这正是 S1 的目的），`chain idle` 7.8–9.3%（`ahead1` 10.0%）。
每笔写 20.4–20.5 KB（`ahead1` 20.6–21.8 KB）。**本轮没有同会话对照臂**（原计划的 `attrib3` 第三臂未跑），
所以这些跨会话差值不作证据。

**复核带出的新线索（已自行复算确认）**：owner 一次调用的固定开销是本轮最大的、且完全在
`aic8800` 与 `ax-driver` 边界内的未分解项——1313–1856 次/秒、括号 130–165 µs/次，占墙钟
**21.7–24.9%**，而其中 `device.advance` 四类合计只占 7.3–8.2 点，余下 **14.4–16.7 点落在 owner 脚手架里**。
`rearm_and_check` 并非纯重武装：`rdif/device/endpoints/startup.rs:242-251` 里它调
`owner.rearm_and_advance(now)`，是一整次 owner 推进。比 `post`（8–9 点）与主机侧每笔准备（约 3.6 点）都大。
**动手前必须先把它分段分解**（flush / latch / advance / submit / rearm / publish），未标定不得优化。
详见执行方案 §11.11。

---

## 待办与下一步（2026-09-29 第五阶段第二轮 S0 后修订）

**本轮新增（按性价比排序）：**

- **A. 补 `attrib3` 第三臂（零代码，最优先）**：`ahead0`/`ahead1` 都含本轮重构，
  它们**互为该重构的对照不成立**。把现成的 `sg2002_starryos_wifi_sta_attrib3_20260929.img`
  当第三臂，在同一热点会话内与 `ahead0`/`ahead1` 用相同用例序列交错 2–3 次，
  一次分清「会话差异」与「本轮重构公共路径的非等价改动」。
- ~~**B. 修两条实现问题**~~**（已完成，见本节上方的 S1 与供给口径修正）**：
  `stage_next_transmit` 的减法位置与 `supply` 漏计 `staged_tx`。
- **C. 判读口径（本轮教训）**：判断 5% 量级的改动**不要用吞吐**——同一镜像两次开机可差 24%
  （`064020` 28.3/28.9 对 `064400` 36.4/34.6），且该差本身来自链路状态变化；
  用机制计数与阶段账。板端 `rssi` 是**下行**量，不能预测上行走廊。


1. **credit 自选等待值不值（已结清，2026-09-29）**：`aic,tx-credit-wait-us=0` 单变量对照已做，
   答案是**保留**（判据落在后一支）。那 16.1% 不是空转：去掉等待后每笔写从 20.9 KB 压到 18.6 KB、
   写周期虽短 12% 但总线腿占比与吞吐都不动；且该旋钮只管薄池分支（`credits 3..=7`），
   **credit 等待里约 78% 属于旋钮够不到的保留量分支**。详见跟踪文档「第四阶段·无等待对照」行与
   执行方案 §10.5。
2. **信道占用读数改源**：现读数（`get_sta_info` 的 `chan_time` 三字段）在本固件上恒零，不能用于判定
   「空口是不是天花板」。厂商真正在用的是 `MM_CHANNEL_SURVEY_IND` 的 `chan_time_ms`/`chan_time_busy_ms`
   （`rwnx_msg_rx.c:292-318`）；改造前不要再引用 `busy`/`txbusy`。
3. **帧年龄**：入环打点、emit 时读出（1 次读时钟/帧），与 credit 统计成对读。round-2 四候选里唯一未做的读数。
4. **热点状态这个第一变量**：同一镜像两遍可差一个数量级，而两遍之间同时重启了热点与板卡。
   先做「只重启热点」与「只重启板卡」两臂，再做「热点在线时长 vs 吞吐」的定时参考测量，
   以及厂商镜像的对照两遍。**在这些结论出来之前，不用吞吐数字做任何轮间对比**。
5. **双向相位需同会话 A/B**：第四阶段第三轮的三种双向读数互不可判（接收形状不同、一次接收塌陷），
   纯上行相位已确认次序改动无处发力；双向要给结论只能成对测。
6. **ARP 表项 300 s 到期事件（板载网络栈，需决定是否处理）**：`net/ax-net` 的
   `NEIGHBOR_TTL = 300 s` 到期后重新解析网关 MAC，期间待发包堆在
   `ETHERNET_MAX_PENDING_PACKETS = 128` 的缓冲里、装满即丢包，解析完成后成批冲刷。
   两轮板测各出现一次，且都落在事件所在的那一个用例里（逐秒吞吐单调下滑）；它不是 AIC 驱动问题，
   是否处理（提前刷新、或排队期间不让 TCP 回退）由上层决定。
7. **该 AP 的 BSS 宽度与信道**：带宽仍未到 40 MHz，且厂商基线不再能证明 AP 支持 40 MHz（见瓶颈文档 §2.16）。
   用第三台设备读该 BSS 的 beacons 宽度/信道，或记录热点的信道设置，成本接近零。
8. **ack 失败计数的口径**：三个会话分别约 15%（HT 会话）、28%（HE 正常遍）、31%（HE 异常遍），
   与吞吐没有单调关系，疑似按聚合/尝试计数。需要与厂商 `rwnx_main.c` 的 STA 信息路径对齐语义后才可作为重传率使用。
9. **已知缺陷与偶发，保留记录**：
   - `device/control.rs` 的 `scan_command` 用 376 字节载荷缓冲，却写到偏移 403 及之后
     （`payload[bssid_offset..bssid_offset + 6]`），任何扫描请求都会越界 panic；需要与厂商
     `struct scan_start_req` 的布局对照后单独修复。
   - 双向用例偶发断链：第一轮第八例板端只剩零星 512 B 写、接收计数为 0、无驱动错误事件，
     PC 侧观测 Wi-Fi 断链；第二轮同一命令正常。按偶发记录。
   - 2026-09-30 一轮无错误、无超时、`deferred=0`；`[wifi-sta-info]` 全程 `he-su`、MCS 5~11、rssi −24~−28 dBm。
   - `memory/dma-api` 的测试在本树跑不起来（**既有问题，与本轮的改动无关**）：
     `cargo test -p dma-api` 默认 feature 下 `tests/test.rs` 因 `contiguous_buffer_pool` 受 `pool`
     feature 门控而编译失败；加上 `--features host-test` 后两个集成目标都能编译，
     但链接时缺 `__SpinOps_acquire`/`__SpinOps_release`（`ax-sync` 的 crate-interface
     `SpinOps` 需要在测试侧提供宿主实现，`memory/buddy-slab-allocator/tests/common/` 有一个可照抄的先例）。
     该 crate 也不在 `scripts/test/std_crates.csv` 中，故 `cargo xtask test` 不会选中它。
     本轮新增的前缀测试放在 `tests/test.rs`，已用临时的宿主 provider 跑通并做过变异验证，
     但在补齐 provider 并登记白名单之前**不构成项目入口下的证据**。

---

## 已作废/降级的方向

- **阶段 2「完成即续发」作为主线**：见周期 P3 结论 1，收益上限约 6%，降为后续小项。
- **「纯上行受上游供帧限制」**：见周期 P3 结论 2，改为驱动侧限速。

## 后续方向（后话，未立项）

在方案范围内把流程调顺之后，进一步考虑**突破主线 aic8800 驱动设计边界**，
以更直接地服务硬件异步（数据相准备等）。当前边界的三个抓手与它们各自的门槛：

| 现有边界 | 突破后能做的事 | 代价 / 前提 |
| --- | --- | --- |
| 单 owner、核心不建线程不阻塞 | 提交与收割分离，允许「多笔在飞」 | 完成匹配、credit 记账、取消语义要重做；设计文档的保证需重述 |
| `active_tx` 单槽，完成即回退 rearm 边界 | 连续提交多笔 CMD53（真正的背靠背） | 与 RX 优先（`#2299`）的平衡要重新论证 |
| 核心不持有 DMA 缓冲（适配层每包准备） | 预置多帧暂存 / ADMA2 描述符链 | 破坏现有分层（core 不依赖 RDIF / `DmaBuffer`） |

立项门槛：先由 probe v2 与阶段 2 的实测说明「准备时间」在每包周期中的占比——
若阶段 2 之后准备仍占主导，说明在主线边界内已到顶，才值得动边界。
