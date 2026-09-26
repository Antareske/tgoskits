# StarryOS eBPF 网络栈监测跟踪

本文件按「改动 → 测试 → 现象」的周期跟踪 eBPF 监测的推进。方案与判据见 `netmon-plan.md`；
历史文档存档在 `archive/`。本文件只做索引式追踪，不重复专文内容。

| 周期 | 日期 | 改动（要解决的问题 / 对应方案） | 测试 | 现象 | 状态 |
| --- | --- | --- | --- | --- | --- |
| H1 | 2026-08-01 | 历史：kprobe 版 wifi_monitor（挂旧驱动 `write_fifo` / `enqueue_data_frame`，已弃用路径） | 实板 iperf3 下行/上行 + 8 桶直方图 | 下行 100% < 300 µs；上行 **94.8% 落在 20–50 ms**（当时上行 82.5 Kbps）→ 判为调度时间片；探针开销 19% | 完成（不可移植） |
| H2 | 2026-09-05 | 历史：netmon Phase 1（按新架构写的全栈挂点 + 8 处 `#[inline(never)]` + `/proc/net/queue`） | 仅静态符号验证（QEMU 冒烟被环境阻塞） | 从未上板；riscv64 运行时验证待做 | 待移植 |
| E1 | 2026-09-24 | 方案 §4 S1：kprobe 注册失败不 panic + 挂点注解 + 搬 netmon | 交叉构建 + QEMU 冒烟 | 构建链打通、符号解析全过、kprobe 修复在真实内核生效；**卡在内核 attach 路径（`stop_machine` 改文本）** | 阶段性收尾 |
| E2 | 2026-09-24 | 按范围收敛：去掉 L3 计数挂点、只留驱动层与 aic8800 优化直接关心的点 | 对齐复核 + QEMU 冒烟 | 注解面 8→6 处；`port_tx` 2 字节对齐被拒（标 optional）；attach 卡点定位到 `patch_kernel_text`→`stop_machine` | 待续 |
| E3 | 2026-09-25 | 定位 attach 卡点根因并修复：内核镜像区在内核地址空间按块映射建立，改文本权限触发块拆分，拆分先清空整块描述符 → 连带解除映射正在执行的代码与陷入向量；修复为可执行内核区按基页映射（`fix(axmm)`），并修掉冒烟判据里匹配不到 CRLF 行尾的锚点（`fix(starry,ebpf)`） | QEMU riscv64 冒烟（`netmon --once`，机器判定） | `cargo xtask starry app qemu` 退出码 0（10.6s）：全部挂点 attach 成功（含 E2 中被拒的 `port_tx`），`NETMON_END` 正常输出；计数为 0 属预期（`--once` 在 attach 后立即快照，本轮无流量） | 完成（待上板） |

## 周期 H1：历史 kprobe 版（背景，不可移植）

### 改动

`sg2002/wifi-ebpf`（提交 `56e2b4119`）：kprobe/kretprobe 靠 `/proc/kallsyms` 符号名直接插桩，
驱动源码零改动。挂点：`fdrv/thread/tx.rs::enqueue_data_frame`、`fdrv/core/sdio_transport.rs::write_fifo`。
应用 `apps/starry/ebpf/wifi_monitor/`：3 个探针、5 个 map、8 桶固定阈值延迟直方图。

同期还有一条 raw tracepoint 线（`sg2002-wifi-ebpf-raw-tracepoint`，`588060224`），
因"对源码改动过大"被放弃，其驱动侧钩子已在该分支上整体删除。

### 现象

- 下行（板端只发 ACK）：`SDIO_WR_LAT 3617 0 0 0 0 0 0 0` → 100% < 300 µs；
- 上行（板端发数据）：`SDIO_WR_LAT 17 0 0 0 0 314 0 0` → **桶 5（20–50 ms）占 314/331 = 94.8%**，
  同轮 iperf3 上行只有 82.5 Kbps；
- 探针开销：3 个 kprobe 激活时 12 → 9.77 Mbps（**约 19%**，rbpf 无 JIT）。

### 对现在的意义

1. 上行卡顿的**机制假设**已有先例：调度时间片（sched-rr 50 ms），与 aic8800 线 round-3
   上行窗口 `period_max` 39–230 ms 同量级（见另一工作树 `wt-sg2002-wifi-opt` 的 `www/sg2002/aic8800/aic8800-optimization-tracker.md` 周期 P2）；
2. 判读方法可照搬：**分布**而非均值、"同一探针两种时机对比"；
3. **不可移植**：挂在重构前路径，dev 上这些符号已不存在；且其 HEAD 的无条件 10 ms RX poll kicker
   被 dev 设计文档显式否决。

## 周期 H2：netmon Phase 1（待移植）

### 改动

未合入分支 `feat/net-enhance`（`3ece4e98c`）：`apps/starry/ebpf/netmon/` 三 crate（loader / eBPF 程序 / 共享常量），
15 个 BPF 程序覆盖 L0–L3 与 wifi 控制面；配套 8 处 `#[inline(never)]` 挂点注解；
另含 `/proc/net/queue`（Phase 0）与 `apps/starry/net-bench/`（板测基建）。

### 测试与现象

只做了**静态符号验证**：构建出的 x86_64 内核 ELF 中 6 个 L1–L3 挂点符号存在且唯一，
SDIO/WiFi 符号确认缺席（该构建不含驱动）。QEMU 冒烟被环境问题阻塞。
**riscv64 实板从未跑过**——这是移植后的第一件事。

## 周期 E1：本分支 S1（进行中）

### 改动

按方案 §4 S1，三件事：

1. **前置修复**：`os/StarryOS/kernel/src/kprobe.rs` 的 `register_kprobe` / `register_kretprobe`
   目前失败时 `.expect()` 直接 panic 内核；改为返回错误并向上传递，
   由 `perf/kprobe.rs` 转成 `StarryError::Unsupported` 返回用户态。
   （历史因 RISC-V 函数序言 AUIPC+JALR 触发 `UnsupportedInstruction` 炸过一次。）
2. **挂点注解**：8 处 `#[inline(never)]`（清单见方案附录 A），dev 现有 0 处。
3. **搬 netmon**：15 个 BPF 程序 + loader + 常量；kallsyms 片段按 dev 的 mangled 名重算；
   部署改为镜像 `--inject` 注入静态 musl 二进制。

### 测试

编译 + clippy；实板冒烟（同 aic8800 线的 STA 镜像流程）：各层计数非零、直方图有分布、
无内核 panic；另跑一轮开/关监测的 iperf3 对比以量化扰动。

### 现象

- **前置修复已完成**并落到本分支（提交 `b7482e652`，`fix(starry): report kprobe registration
  failure instead of panicking`）：两个注册函数改为返回 `StarryError::Unsupported` 并记录被拒的挂点，
  `perf_event_open` 路径向上传递，`kprobe_test` LKM 改为报告拒绝而非假定成功。
  验证：SG2002 内核构建通过（`-D warnings`）、`cargo xtask starry kmod build --arch riscv64`
  产出 `kprobe_test.ko` 通过。
- **挂点注解已完成**（8 处 `#[inline(never)]`，见下方小节），并**从带注解的内核 ELF 实测导出挂点符号**：
  9 个目标函数里 8 个解析为唯一符号，`port_tx` 因闭包/shim 共用路径有 3 个匹配
  （已给 loader 加「优先 `_RNv` 函数本体符号」规则）。
- **netmon 搬运完成但构建链被卡住**（见下方「构建链阻塞」）。

### 挂点注解（提交待做）

在 dev 树上新增 8 处 `#[inline(never)]`（注解前 `count_tx`/`count_rx`/`submit_*_dma` 等符号
在 release ELF 里**确实不存在**，印证了历史教训「release 内联消灭符号」）：

| 文件 | 函数 |
| --- | --- |
| `drivers/blk/sdmmc-protocol/src/sdio/io/transfer.rs` | `SdioCard::submit_read_dma` / `submit_write_dma` |
| `drivers/net/aic8800/src/rdif/device/endpoints/control.rs` | `AicWifiControl::start`（`WifiControl` 实现） |
| `net/ax-net/src/queue_runtime/executor/mod.rs` | `QueueFramePort::transmit` / `receive`、`QueueGroupExecutor::poll` |
| `net/ax-net/src/queue_runtime/state.rs` | `PollGroupState::schedule_irq` |
| `net/ax-net/src/router.rs` | `DeviceHandle::count_tx` / `count_rx` |

符号校验（对 `target/riscv64gc-unknown-none-elf/release/starryos` 用 `nm` 逐条核对）：
`count_tx` / `count_rx` / `schedule_irq` / `queue_poll` / `port_rx` / `sdio_read` / `sdio_write` /
`wifi_start` 均**恰好 1 个**匹配；`port_tx` 匹配 3 个（闭包 + `FnOnce` shim），
故 loader 新增规则：多重匹配时收敛到以 `_RNv` 开头（值命名空间，即函数本体）的那个，
仍不唯一则报错。

### 构建链阻塞（需要环境决策）

`cargo build --target riscv64gc-unknown-linux-musl`（loader 交叉编译 + eBPF 程序链接）失败，
原因是一条**版本链**：

1. eBPF 程序由 aya-build 用 `nightly` 工具链编译，产出的 LLVM 位码版本取决于该 rustc：
   本机默认 `nightly-2026-09-04` 的 LLVM 是 **23.1.1**；
2. 本机安装的 `bpf-linker 0.9.15` 内嵌 **LLVM 19.1.1**，读不了 LLVM 23 的位码：
   `Unknown attribute kind (102) (Producer: LLVM23.1.1 Reader: LLVM 19.1.1)`；
3. 「换旧工具链」这条路走不通：项目根的 `.cargo/config.toml` 用了 `include = [{path=...}]`
   这个**新配置键**，LLVM 19 时代的 cargo（1.86/1.87 nightly 试过）**连配置都解析不了**
   （`failed to parse key 'include'`），而它是被追踪的项目文件、不能为本地构建改动。

**已按方案 A 修好（2026-09-24）**：加 apt.llvm.org 源装 `llvm-23-dev`（23.1.2，与项目 nightly 的 LLVM 23.1.1 同主版本）
→ 用官方**预编译** bpf-linker 0.11.1（`bpf-linker-x86_64-unknown-linux-musl`，musl 静态、内嵌 LLVM；替换原 0.9.15 并备份为
`/opt/cargo/bin/bpf-linker-0.9.15-llvm19`）→ app 用默认 `nightly`（LLVM 23）即可，无需额外 Rust 工具链。
（`cargo install bpf-linker` 那条路走不通：llvm-sys 的库发现只找 `/usr/lib64`，找不到 libLLVM。）

### 上板前的验证结果（QEMU riscv64，2026-09-24）

流程改用 dev 现代表单：`qemu-riscv64.toml` 重写为 `[[shell_check_steps]]`（旧 `shell_prefix` 已被 xtask 移除）、
`shell_cmd = "/usr/bin/netmon --once"`、`success_regex = ["^NETMON_END$"]`；恢复 `prebuild.sh`（构建 + overlay 安装，去掉 loongarch 特例）。

| 项 | 结果 |
| --- | --- |
| 交叉构建 | ✅ riscv64 musl 静态二进制（3.0 MB）+ 内嵌 BPF 对象（171 KB） |
| 符号解析 | ✅ 在真实 QEMU 内核里逐个 `resolved`（count_tx/count_rx/sched_irq/queue_poll/queue_poll_ret/port_tx/port_tx_ret/port_rx/port_rx_ret）；SDIO/WiFi 探针按 optional 跳过 |
| kprobe 前置修复 | ✅ **真实生效**：注册被拒时返回 ENOSYS 给用户态，内核没有 panic（修复前会炸内核） |
| optional 降级 | ✅ 新增：符号存在但 attach 被拒时 optional 探针降级告警，只有 required 探针才致命 |
| attach | ❌ 未成功：`perf_event_open` → `kprobe registration rejected: InvalidAddress` |
| 内核告警 | `BPF_BTF_LOAD`/`BPF_LINK_CREATE` unsupported（aya 0.13+ 会尝试，日志显示为**非致命**告警）；`CPUMAP`/`DEVMAP` 未实现（aya 能力探测，无害） |

### E2：attach 失败已定位到两个原因（2026-09-24）

**原因 1：挂点落在 RVC（压缩指令）函数的 2 字节对齐入口，被 kprobe 层判 `InvalidAddress`。**

证据链：kprobe crate 的 RISC-V 分支在「读到的 16 位看起来是 32 位指令」且 `address & 0x3 != 0` 时返回
`InvalidAddress`；内核 `lookup_symbol_addr` 走的是 in-kernel kallsyms，**地址与 ELF 符号表完全一致**
（`nm` 核对：`count_tx` @ `ffffffff8015e212`、`schedule_irq` @ `ffffffff80160d8c`，两者相同）；
把 9 个挂点的对齐逐个算出来，恰好就是失败的那两个不对齐：

| 挂点 | 地址 | 4 字节对齐 | 结果 |
| --- | --- | --- | --- |
| `count_tx` | `…e212` | ✗ | attach 被拒（InvalidAddress） |
| `count_rx` | `…c54a` | ✗ | attach 被拒 |
| `sched_irq` | `…0d8c` | ✓ | 待验证 |
| `queue_poll` | `…e4bc` | ✓ | 待验证 |
| `port_rx` | `…2dc0` | ✓ | 待验证 |
| `port_tx` | （3 个候选，闭包/shim） | — | loader 的 `_RNv` 规则收敛 |

处置：L3 的 `count_tx`/`count_rx` 标为 optional（它们的数据 `/proc/net/dev` 本来就有，不用 eBPF 也拿得到）；
选挂点时**优先 4 字节对齐的大函数**。

**原因 2：越过两个被拒的探针后，运行卡在第一个对齐探针（`sched_irq`）上，无 panic、无后续输出。**

位置待定，两个候选：
- 内核侧 `patch_kernel_text` / TLB 同步（`kprobe.rs` 的 `set_writeable_for_address` 内核分支；
  注意那里仍是 `.expect(...)`，即**打补丁失败会 panic 内核**，与前面刚修掉的注册路径是同一类风险）；
- `alloc_kernel_exec_memory`（分配可执行页）。

**范围收敛（按用户要求，2026-09-24）**：/proc 与应用态已有的每接口计数不再挂 ——
删除 `count_tx`/`count_rx` 两个 BPF 程序、loader 声明与对应常量槽，并**撤回 `router.rs` 的两处注解**
（`feat(ax-net,drivers)` 提交 amend 后为 `1a6ec133a`）。注解面从 8 处/5 文件/+18 行收敛到
**6 处/4 文件/+14 行**，只保留：L2 队列边界、L1 IRQ/poll、L0 SDIO 的 CMD53、WiFi 控制面。

**剩余挂点的对齐复核**（QEMU 内核）：`sched_irq` ✓、`queue_poll` ✓、`port_rx` ✓ 均 4 字节对齐；
`port_tx` 落在 `…336`（2 字节对齐）✗ → 标为 optional，attach 被拒时自动降级。
**待办**：驱动层三个挂点（`submit_read_dma`/`submit_write_dma`/`AicWifiControl::start`）的地址对齐
需在**板级内核构建**上复核（当前 target 目录里是 QEMU 构建，没有这些符号）——上板前必做，避免白跑一轮。

**卡点定位（原因 2 更新）**：去掉 L3 后重跑，日志在 `resolved` 之后就停住（无 `attach rejected`、无 panic）
→ 卡在**第一个对齐挂点 `sched_irq` 的 attach** 里。核内核代码：内核分支的
`set_writeable_for_address` → `crate::mm::patch_kernel_text` → **`crate::stop_machine::stop_machine(...)`**
（另一个候选是 `alloc_kernel_exec_memory` 的 `allocate_kernel_range`）。这两处失败时都是 `.expect(...)`，
即 panic 而非卡死，故更像是 `stop_machine` 阶段阻塞。

**RVC 之谜已解开**：用 `llvm-objdump` 反汇编两个失败地址——`count_tx` @ `…e212` 处是一条
32 位指令（`lbu t3,0x5c(s2)`，低两位 `11`），而它落在 2 字节边界上；同函数前后都是 32 位指令序列。
即**符号地址本身不在指令边界上**（比函数真正入口多 2）。所以探针层判 `InvalidAddress` 是对的，
不是 crate/内核的 bug；这两个挂点确实不可用（已标 optional）。同类情况以后用同样方法复核即可。

**卡点定位（两轮打点）**：加了 10 处 `[kprobe-diag]`（`register_kprobe` 包装、符号解析、
exec 内存分配、写权限、`patch_kernel_text` 闭包内外的 stop_machine 进出）——**一处都没打印**，
而内核 ELF 里确实含这些字符串（`strings` 计数 10）。所以卡点在**我们内核的 kprobe 代码之前**：
aya 用户态 `load()`/`attach()` 的前置步骤、或 syscall 入口/`perf_event_open` 的分发阶段。
下一步：在 loader 里加用户态打点（`load` 前后、每个 attach 前后带程序名），一次 QEMU 周期即可二分。

**方案分叉（待定）**：

| 方案 | 原理 | 代价 | 判定 |
| --- | --- | --- | --- |
| B1 继续修 kprobe（现状） | kprobe 用 `ebreak` 替换入口指令、单步执行原指令；必须改内核文本（`stop_machine` + 权限 + icache 同步） | 依赖内核细节，深度未知；通了则 netmon 全部挂点可用、源码侵入最小 | **已采用并跑通**：卡点在文本改权限引发的块映射拆分，修好映射粒度即可（周期 E3）；2026-09-25 QEMU 冒烟全部挂点 attach 成功 |
| B2 改用内核已支持的 `raw_tracepoint`（`BPF_RAW_TRACEPOINT_OPEN` 已实现） | 在关心的位置预置**静态 tracepoint**（关闭时是 nop 级开销），BPF 挂 tracepoint 而非改指令 | 需在内核/驱动源码加 tracepoint 定义与调用点（源码侵入，但 Linux 的常规做法） | 机制最稳、开销最低，避开改文本/对齐/stop_machine |
| B3 内核内直方图（把临时探针升级为常驻设施） | 钩子点直接读时钟、写 per-CPU log2 直方图，无新机制 | 每次改动要重编译；每个点几行侵入；不是通用工具 | 今天就能用，作为保底（aic8800 线的 A0 直方图同思路） |

候选解法（未采用，留档）：

| 方案 | 做法 | 代价 |
| --- | --- | --- |
| A（推荐） | 升 `bpf-linker` 到 0.11.1（支持 LLVM 21/22/23）+ 装匹配的 LLVM dev 包 + eBPF 侧工具链固定到同 LLVM 版本的 nightly | 需加 apt 源并安装 LLVM dev（数百 MB）、`cargo install bpf-linker`、再装一个 nightly；app 的 `build.rs` 里把 toolchain 固定下来 |
| B | 把 eBPF 侧固定在 LLVM 19（沿用现成 bpf-linker 0.9.15），构建时把 app **拷到仓库外**再编（绕开项目 config） | 不需要系统改动，但构建流程带一份「拷出去编」的临时步骤，不可复现于仓库内 |

顺带得到一个可能的解释：历史那条线**从未真正跑到板上**，可能不只是"环境问题"，
构建链在当时的工具链/配置组合下就未必能建成。

## 周期 E3：attach 卡点根因与修复（2026-09-25）

### 改动

`os/arceos/modules/axmm/src/lib.rs`：构建内核地址空间时，`memory_regions()` 里的
**可执行区**（内核镜像区，`MemRegionFlags::EXECUTE`）改走 `AddrSpace::map_linear`
（可变线性 backend，基页），其余区仍走 `map_boot_linear`（允许块映射）。
两个 backend 在 map/unmap/validate 上完全同构，唯一差异就是"是否允许块映射"。

### 定位过程

1. loader 侧打点：卡点在 `perf_event_open` 内部、`register_kprobe` 之前（探针程序已 load 成功）；
2. 宿主侧复现 kallsyms 查找（抽取内核 ELF 的 `.kallsyms` 段 + `ksym` 现成跑）：
   10333 个符号回查仅 1 个超长名字失败，探针用名全部正常 → **排除符号解析**；
3. QEMU gdb stub 取样（4/4 次一致）：`pc` 停在 `trap_vector_base`，`ra` 在
   `split_leaf_for_boundary+1102`，`scause=12`（指令页错误），**`stval` 等于陷入向量自身地址**
   → 陷入向量所在页已被解除映射，CPU 取指即再次陷入，形成无限陷入风暴；
4. 读码定位：可执行区由 `map_boot_linear`（`allow_huge: true`）建立，`patch_kernel_text`
   改权限 → `protect_region` → `split_leaf_for_boundary` → 拆分 block 映射；
   拆分的 break-before-make 先 `clear()` 整个块描述符，而该块同时装着正在执行的代码与陷入向量。

### 判读与教训

- **块拆分对"自己所在的映射"是自毁操作**：只要内核文本是块映射，任何改权限都会踩到这条路径；
  与架构无关，只要内核会改自己文本的权限就成立（当前只在 riscv64 上被触发并验证）。
- **卡死后内核日志不再输出，不等于代码没执行**：日志记录是非阻塞入队，串口 worker 被饿死后
  记录只被丢弃，因此"打点没打印"不能推出"没走到"——E1/E2 的 10 处打点没输出即属此类。
  这类场景要用同步输出（gdb/emergency console）而不是普通日志。
- `BootLinear` 的文档本身就写明 "immutable … must not be partially unmapped"，
  而 `patch_kernel_text` 正是部分改权限——原先的映射方式与该约束冲突。
- 改动落在共享的 `axmm`，因此另跑一轮跨架构回归：aarch64 QEMU + `ebpf/syscall_count`
  启动与 eBPF 功能正常（`SYSCALL_COUNT_PASS`，退出码 0）。

## 待办与下一步

1. **上板冒烟**：板测镜像已出（`sg2002_starryos_wifi_ebpf_netmon_20260925.img`：
   本分支内核 + 复用现成 STA/iperf3 rootfs + 注入 `/usr/bin/netmon`，boot.sd 校验通过），
   待烧写后跑开/关监测两组 iperf3。
   板级内核构建（`licheerv-nano-sg2002-wifi.toml`）已复核：7 个挂点全部唯一解析，
   **入口指令都是 16 位**，探针层走 Inst16 路径 → 地址是否 4 字节对齐不构成拒绝条件
   （复核方法见 `check-hook-alignment.py`；判据来自 kprobe crate 的 rv64 分支：只有入口
   编码为 32 位指令且地址非 4 字节对齐时才返回 `InvalidAddress`）；
2. **S2**：新增四个挂点（`take_tx_frame` 逗留、owner 步耗时、核心 emit/完成配对、
   SDHCI 中断侧拆分），产出直接服务 aic8800 线的 P3/A3；
3. **待定**：`/proc/net/queue` 的去留（属 proc 集成，但其计数是唤醒/调度侧证据）；
4. **明确不做**：net-bench、CI 断言、socket 层与 raw tracepoint 两代。

## 复现件索引

`repro/` 存放本轮定位用的宿主侧复现与 gdb 取样脚本（含 gdb 取样判读表），
不参与项目追踪。

## 与 aic8800 优化线的交汇

同一 worktree 内两条线并行开发、按提交主题区分（`fix(starry)` / `feat(starry,ebpf)` / `perf(aic8800)`），
未来拆分成各自的分支与 PR。交汇点：

- eBPF 的 **L1 唤醒延迟**与**行内 CMD53 时长分布** → 判定 aic8800 线 A0（上行是调度切片还是拉取慢）；
- eBPF 的 **`take_tx_frame` 逗留时间** → 回答 aic8800 线 P3（帧在等拉 vs 栈没产帧）；
- 反过来，aic8800 线的每次阶段改动都给 eBPF 提供一次"改动前后对比"的机会。

## 阶段性收尾（2026-09-24）

### 已完成

- **构建链打通**：`llvm-23-dev` + 预编译 `bpf-linker` 0.11.1 + aya 钉在走 `perf_event_open` 的 rev；
  riscv64 musl 静态 loader（3.1 MB）+ 内嵌 BPF 对象可产出；
- **loader 在真实内核里验证**：9 个候选挂点全部解析成功（SDIO/WiFi 在 virtio 镜像里按 optional 跳过）；
- **kprobe 前置修复在真实场景生效**：注册被拒 → ENOSYS 返回用户态，不再 panic 内核；
- **optional 降级**：符号在但 attach 被拒时按层降级告警，只有 required 探针才致命；
- **范围收敛**：不挂 /proc 与应用态已有的计数；注解面 6 处/4 文件/+14 行。

### 未完成（下一步的起点）

1. **内核 attach 卡点**（阻塞一切）：第一个对齐挂点 `sched_irq` 的注册过程卡住，无 panic。
   路径 `set_writeable_for_address` → `mm::patch_kernel_text` → `stop_machine`，
   备选 `alloc_kernel_exec_memory` 的 `allocate_kernel_range`。两处失败都是 `.expect(...)`（会 panic 而非卡住），
   故更可能是 `stop_machine` 阶段阻塞。**建议先写个最小 LKM 直接调一次 `patch_kernel_text`** 来区分。
   顺带：这两处的 `.expect` 与刚修掉的注册路径同类，值得一并改成可返回错误。
2. **RVC 入口支持**：`count_tx`/`count_rx`/`port_tx` 落在 2 字节对齐（压缩指令）入口，被探针层判
   `InvalidAddress`。Linux 的 kprobe 支持压缩入口，值得对照补上；补上后这三个挂点可恢复。
3. **驱动层挂点的对齐复核**：`submit_read_dma`/`submit_write_dma`/`AicWifiControl::start` 要在**板级构建**上
   确认 4 字节对齐（当前 target 目录是 QEMU 构建，缺这些符号）——上板前必做。
4. **板测镜像**：attach 修好后出镜像，跑开/关监测两组 iperf3 量化扰动。

### 环境改动留档（按方案 A 执行）

- 新增 apt 源 `apt.llvm.org`，安装 `llvm-23-dev`（23.1.2）；
- `bpf-linker` 换成官方预编译 0.11.1（musl 静态），旧版备份在 `/opt/cargo/bin/bpf-linker-0.9.15-llvm19`；
- 新增 Rust 工具链：`nightly`（aya-build 实际使用）、`nightly-2025-02-20`、`nightly-2025-01-15`
  （后两个在排查中试过，**已不再需要**）。
