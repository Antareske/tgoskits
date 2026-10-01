# StarryOS eBPF 实现经验总结

基于 wifi_monitor (kprobe/kretprobe) 在 SG2002 RISC-V 上的开发经历。面向后续 eBPF 工作，记录踩过的坑和关键决策点。

## 一、架构相关

### 1. `bpf_target_arch` 必须显式设置

**问题**：aya-build 的 `emit_bpf_target_arch_cfg()` 在没有显式设置时回退到 `HOST` 架构（即构建机的 x86_64）。eBPF 字节码中的 `ProbeContext::arg(n)` 和 `RetProbeContext::ret()` 都用架构特定的寄存器偏移——用 x86_64 偏移读 RISC-V 的 pt_regs，`arg(1)` 读到垃圾值，所有计数器静默为 0。

**解决**：在 prebuild.sh 构建 eBPF 之前 `export AYA_BPF_TARGET_ARCH=riscv64`。

**教训**：所有非 x86_64 目标的 eBPF 程序都要设这个环境变量，否则编译通过、挂载成功、计数器全零——没有任何报错提示。

### 2. RISC-V kprobe 不支持部分入口指令

**问题**：`sdhci_irq_handler` 的函数入口指令被 kprobe crate 判定为 `UnsupportedInstruction`，无法植入断点。在 RISC-V 上函数序言常见 AUIPC + JALR 组合，kprobe 的指令搬迁解码器不认。原来的实现用 `.expect()` 直接 panic 内核。

**解决**：两步：(1) 将 `register_kprobe`/`register_kretprobe` 从 `.expect()` 改为返回 `AxError`，让错误传到用户态而非炸内核；(2) 放弃挂该符号（IRQ 计数在 SDHCI 驱动内部已有计数器，eBPF 冗余）。

**教训**：RISC-V 上 kprobe 的指令兼容性不如 x86_64，新探针目标应优先选择"大函数"（编译后指令多、入口指令更可能是标准序言），小函数或 D1 入口容易踩坑。

### 3. Vec<u8> 在 RISC-V Rust ABI 下的传参方式

**问题**：`enqueue_data_frame(bus: &Arc<WifiBus>, eth_frame: Vec<u8>)` 中 Vec<u8> 是 24 字节，超过 2×XLEN(16)，RISC-V psABI 下按隐藏指针（byval）传递，a1 指向调用者栈上的 `{ptr, cap, len}` 结构体。`ctx.arg(1)` + `vp + 16` 读 len 是正确的。

**无关的担忧**：曾怀疑 Vec 拆为 3 寄存器分别传 ptr/cap/len——这对 ≤16 字节的聚合体成立，对 24 字节的 Vec 不成立。用 `rustc --emit=llvm-ir` 或反汇编确认比猜 ABI 可靠得多。

**教训**：eBPF 探针读函数参数时，必须对照目标架构的 ABI 和实际编译产物验证 `ctx.arg(N)` 索引，不能凭经验或 x86_64 习惯推断。Rust ABI 对大型聚合体的 byval 处理在不同架构上有一致性（都是超过 2×XLEN 走指针），但具体阈值因 XLEN 而异。

### 4. kprobe 注册失败应优雅报错而非 panic

**问题**：原 `register_kprobe` 用 `.expect("Failed to register kprobe")`，探针挂载失败直接炸内核。

**解决**：改为 `AxResult`，返回 `AxError::Unsupported` 并通过 `perf_event_open` 系统调用返回给用户态。aya loader 收到 errno 后打印错误退出，内核存活。

**教训**：探针注册是运行时操作，失败原因多样（指令不兼容、符号不存在、地址越界），不应该用 panic 处理。所有内核侧的探针注册路径都应该可恢复。

## 二、符号解析

### 5. Rust v0 修饰名与子串匹配

**问题**：aya loader 通过 `/proc/kallsyms` 查找符号。StarryOS 使用 `rust-nm` 提取内核 ELF 的符号表，保留原始 Rust v0 修饰名。v0 修饰名以明文嵌入模块路径、类型名和方法名（形如 `_RNv...aic8800...sdio_transport...SdioTransport...write_fifo`），可以用子串匹配消歧。

**解决**：`resolve_symbol_name(&["write_fifo", "sdio_transport", "SdioTransport"])` 用三子串 AND 匹配，精确区分 aic8800 的 `SdioTransport::write_fifo` 和 sdhci-cv1800 的 `SdioHost::write_fifo`。

**教训**：(1) v0 修饰名中的标识符在 4K 符号长度限制内不会截断；(2) 应加上匹配数 != 1 时报错而非取第一项，避免将来新增同名符号时静默错挂；(3) 子串 AND 语义要明确（`.all()` vs `.any()`）。

### 6. read_fifo 符号不存在的根因

**问题**：sg2002 上 `SdioTransport::read_fifo` 不在 kallsyms 中。实际调用链通过 trait 分发到 `SdioHost::read_fifo`，后者符号名不含 `SdioTransport` 子串。

**误解**：最初以为是函数被内联优化掉了。实际上该文件/模块路径的条件编译导致 `SdioTransport` 版本根本没有独立符号实体。

**教训**：先用 `rust-nm` 确认符号是否存在于编译产物中，再分析不存在的根因（内联 vs 未编译 vs trait 分发），不要跳过实证直接假设。

## 三、构建系统

### 7. prebuild.sh 的 musl 静态二进制模式

**事实**：所有 StarryOS eBPF app（kret、sched_trace 等）都通过 prebuild.sh 交叉编译为 `<arch>-unknown-linux-musl` 静态二进制，注入 rootfs overlay。StarryOS 实现 Linux 兼容的 syscall ABI（bpf、perf_event_open、ioctl 等），可以直接运行 musl 静态二进制。

**启示**：这就是 StarryOS eBPF 的标准部署路径，不需要改成 StarryOS 原生 target。

### 8. rustup target add 可能因 CDN/版本问题失败

**问题**：nightly-2026-07-15 的 `riscv64gc-unknown-linux-musl` target 下载反复失败（"cleaning up cached downloads"），而 nightly-2026-05-28 可用。

**解决**：从旧 toolchain 复制 rustlib 不可行（std rlib 元数据哈希绑定编译器版本，跨版本链接出错）。正确做法是换可用 toolchain 或等 CDN 修复。prebuild.sh 中的 fallback 拷贝逻辑应删除——它只会在同版本 toolchain 别名时偶然可用，其余情况必然失败且有写入根目录的风险。

**教训**：prebuild.sh 不宜包含"猜测性恢复"逻辑，失败时给出清晰的错误信息比尝试自动修复更可靠。

### 9. rootfs 包安装需要 qemu-user-static

**问题**：需要在构建机上往 riscv64 rootfs 镜像安装 iperf3，但无法直接 chroot（架构不同）。

**解决**：用 `debugfs rdump` 导出 rootfs → `cp qemu-riscv64-static` → `chroot` + `apk add iperf3` → `debugfs write` 回写关键文件。

**教训**：WSL 环境下没有 loop 设备支持无法 mount ext4 镜像，但 debugfs rdump/write 可以覆盖大多数文件注入需求。跨架构包管理是常规操作。

## 四、eBPF 程序设计

### 10. kprobe/kretprobe 之间的状态传递

**问题**：kprobe 入口记录时间戳和长度，kretprobe 出口读取并计算延迟。只能在 eBPF map 中以全局 key 传递——没有 per-thread 上下文。

**限制**：当前用单槽 `WR_ENTRY{0=ts, 1=len}`，多线程并发调用时入口值可能被覆写。对 TX 数据路径 + WPA2 握手 + 初始化的并发场景，这是真实问题。

**未在此轮修复**：per-thread 隔离需要 `bpf_get_current_pid_tgid()` 的低 32 位 tid 作为 map key 前缀，涉及 tid 回收和 map 容量管理，非 trivial。

**教训**：设计 kprobe/kretprobe 对时，应从一开始就用 per-thread key 方案，而不是先做单槽再改——改动的风险（引入新竞态、破坏已有语义）高于一开始就做对。

### 11. eBPF 侧的静默错误

**问题**：`bpf_probe_read_kernel(...).unwrap_or(0)` 返回 0 时，frame_len == 0 路径什么都不计数，没有任何方式知道"读成功了但长度是 0"还是"读失败了返回默认值"。同样，`let _ = map.insert(...)` 丢弃了 map 操作的全部错误信息。

**教训**：eBPF 中所有 fallible 操作都应考虑"静默吞错"的后果。无条件计数器（临时 debug 用）是有效的诊断手段——至少能区分"探针没触发"和"触发了但逻辑跳过了"。

### 12. 探针对吞吐的开销

**实测**：3 个 kprobe 激活时，iperf3 吞吐从 ~12 Mbps 降至 ~9.77 Mbps，开销约 19%。这部分来自每次 SDIO 写入触发：kprobe 断点 → ebpf 解释器执行 → kretprobe 断点 → ebpf 解释器执行（rbpf 无 JIT，纯解释）。

**启示**：RISC-V 上无 JIT 时开销会更明显。探针数量应克制——只保留对诊断目标必需的探针。

## 五、运维与测试

### 13. 单次采样 vs 持续监控

**问题**：最初设计为 `loop { sleep(5s); dump }` 持续监控。后改为单次采样模式 `sleep(N)` → dump → 退出，探针随进程退出自动卸载。

**模式选择**：
- 持续模式：适合长期后台监控，但需要信号处理和优雅退出
- 单次模式：适合"开窗观测一段流量然后退出"的测试场景，使用更简单

**当前实现**：`wifi_monitor <秒数>` 后台运行 + 前台跑 iperf3 + 等待 wifi_monitor 到期输出。

### 14. 测试时的横向对比

正常场景和崩塌场景的延迟直方图对比极具诊断价值：正常时 100% < 300µs；崩塌时 94.8% 在 20-50ms（恰好一个 sched-rr 时间片）。这种"同一个探针、不同时机"的对比，比单次绝对值有说服力得多。

**启示**：设计测试流程时应明确构造"正常基线"和"问题复现"两个场景，用同一套探针分别采集然后对比。

### 15. 不需要的功能就删掉

**本次删除的**：read_fifo（符号不存在）、sdhci_irq_handler（UnsupportedInstruction + 已有内核计数器冗余）、持续 loop 模式（改为单次采样）。

**逻辑**：挂不上就跳过，有替代就删除，不保留"将来可能有用"的代码。eBPF 探针本身就有性能开销，保留无用的探针是净负债。

### 16. 核心结论

| 问题 | 根因 | 解法 |
|------|------|------|
| 计数器全零 | AYA_BPF_TARGET_ARCH 未设 | prebuild.sh 设 `export AYA_BPF_TARGET_ARCH=riscv64` |
| irq_handler 挂不上 | RISC-V kprobe 不支持入口指令 | 删掉，已有内核计数器 |
| read_fifo 找不到 | 符号不在 SdioTransport 上 | 删掉，TX 写延迟已覆盖核心诊断维度的 |
| kprobe 注册失败炸内核 | expect() panic | 改 AxResult 返回错误 |
| 采样窗口无数据 | 单次打印在流量开始前就退出了 | 加 sleep 参数，后台运行 |
| 调试代码残留 | 临时计数器未清理 | 提交前清理所有临时 inc_map |
