PVH 错误根因定位
错误现象
qemu-system-x86_64: Error loading uncompressed kernel without PVH ELF Note
QEMU 用 -kernel <ELF> 直接加载 x86_64 内核时，要求该 ELF 内含 PVH ELF Note（x86 paravirt 直接启动入口约定）。报错说明被加载的 target/.../release/starryos 是一个没有 PVH note 的普通 ELF。
根因链（逐层证据）
1. 默认 dynamic 平台：build.rs:1197 default_plat_dyn() -> true。nginx 未显式关闭，故 x86_64 走 dynamic 平台，构建带 plat-dyn feature（运行日志中 --features ...,plat-dyn,smp,plat-dyn 已证实）。
2. dynamic x86_64 需要 UEFI + BIN 启动：正确路径由 apply_dynamic_platform_qemu_boot（test/qemu.rs:1000）程序化设置：
- qemu.uefi = true、qemu.to_bin = true（1015-1016 行）
- 仅 x86_64 的总线/默认设备/五级分页/嵌套虚拟化/调试参数调整（1023-1028 行）
触发条件 cargo_dynamic_platform_boot_arch（1244 行）：feature 含 plat-dyn 且 target 为 x86_64-unknown-none → 返回 X86_64。nginx 完全满足。
3. app-qemu 派发路径从不调用该函数：
- nginx 走 starry app qemu → app_qemu()（starry/mod.rs:508）
- 两条出口（行 574、627）都直接调用 self.app.qemu(...) 即 Context::qemu()（context/mod.rs:141）
- Context::qemu() 第 149-151 行只调用 apply_x86_64_kvm_accel_if_available，没有 apply_dynamic_platform_qemu_boot
- 对比 test/rootfs 路径（starry/rootfs.rs:114、starry/test.rs:943、arceos/test.rs:1440 等）全部调用了该函数
4. 结果：dynamic x86_64 内核在 app-qemu 下既没设 uefi=true，也没 to_bin=true。x86 qemu 配置里 to_bin = false（apps/starry/nginx/qemu-x86_64.toml:15），于是 QEMU 以纯 -kernel <ELF> 加载 dynamic 内核。运行日志末行证实：
qemu-system-x86_64 ... -kernel .../x86_64-unknown-linux-musl/release/starryos
没有 .bin、没有 OVMF/pflash → ELF 缺 PVH note → 报错。
为什么是框架层、与 nginx 无关
- 触发条件只取决于「dynamic 平台 + x86_64 target + app-qemu 派发」，与具体 app 无关；last-commits.md 已用未改动的 redis 复现同样报错。
- 仅靠测例 toml 无法修复：UEFI 总线、关五级分页、关默认设备、嵌套虚拟化等是 apply_dynamic_platform_qemu_boot 运行时程序化生成的，toml 无对应字段；只写 uefi=true/to_bin=true 得到的是残缺配置，仍起不来。
修复方向（属另一个 PR / scripts/axbuild）
在 app-qemu 派发路径补一次 apply_dynamic_platform_qemu_boot。最贴合的落点是 Context::qemu()（context/mod.rs:149-151）——把现有的：
if let Some(qemu) = qemu.as_mut() {
    crate::test::qemu::apply_x86_64_kvm_accel_if_available(qemu, &cargo);
}
改为调用 apply_dynamic_platform_qemu_boot(qemu, &cargo)（它内部已先调 KVM 探测，再按 dynamic arch 设 uefi/to_bin 及 x86 专属调整），即可让所有走 Context::qemu() 的 dynamic x86_64/loongarch64 路径与 test 路径行为一致。这属于 app-qemu 框架层问题，独立于 nginx 测试入口组织。







结论：不能简单靠 to_bin=false 让 x86 走 PVH 启动
x86_64 dynamic 平台的 someboot 启动路径架构上就是 UEFI PE/EFISTUB 镜像，不是 PVH 入口。理由（全部代码实证）：
1. 入口是 PE 头 + EFI stub，不是 PVH
- arch/x86_64/head.rs 的 _head 是手写的 PE/COFF 头（IMAGE_DOS_SIGNATURE、LINUX_PE_MAGIC、IMAGE_SUBSYSTEM_EFI_APPLICATION），入口指向 __x86_64_efi_pe_entry。这是 Linux EFISTUB 风格的 UEFI 可执行镜像。
- link.ld 里虽有 note PT_NOTE 段，但 grep 全仓库没有任何 PVH ELF Note（XEN_ELFNOTE_PHYS32_ENTRY 等）的定义。QEMU 的 PVH 直接启动恰恰要求这个 note —— 没有它，-kernel <ELF> 必然报 Error loading uncompressed kernel without PVH ELF Note。
2. 启动信息（内存图/ACPI）依赖 UEFI Boot Services
- kernel_entry（entry.rs:23）在 efi_boot != 0 时才 setup_service + 后续 exit_boot_services()。
- 内存图来源是 fdt::init_memory_map()，而 x86_64 的 FDT_ADDR 只在 efi_stub::find_fdt() 里通过 UEFI ConfigTable 设置（efi_stub/mod.rs:315）。riscv64 从启动寄存器拿 FDT，x86_64 没有非 UEFI 的 FDT/e820 来源。
- 也就是说，即使内核能从 PVH 入口跑起来，它也拿不到内存图（ram::init 会 expect("No free memory") panic）和 ACPI。
3. PVH 启动需要消费 hvm_start_info，当前完全没有实现
PVH 协议下 QEMU 通过 %ebx 传 hvm_start_info（含 memmap、cmdline、ACPI RSDP）。someboot 没有任何解析它的代码。
让 x86 支持 PVH 需要做的工作（非小改）
如果真要走 PVH（to_bin=false、纯 -kernel、不依赖 OVMF）：
1. 新增 PVH ELF Note：在 .note 段写 Xen namespace 的 XEN_ELFNOTE_PHYS32_ENTRY，指向一个 32 位保护模式入口。
2. 新增 32 位 PVH 入口代码：从 32 位 flat protected mode 开始，建立长模式、初始页表、栈，再跳到 Rust。这是与现有 _head(PE/UEFI) 并存的第二条裸启动入口。
3. 解析 hvm_start_info：把 PVH memmap/cmdline/RSDP 转成 someboot 现有的 MemoryMap/FDT 抽象（或新增非 UEFI 的内存图来源）。
4. ACPI 非 UEFI 发现：当前 ACPI earlycon 也走 UEFI/FDT，需要从 PVH RSDP 或 BIOS 扫描兜底。
这等于给 x86_64 加一条全新的裸金属启动路径，blast radius 大，属于 someboot/somehal 的架构级改动。
对当前 nginx PR 的实际意义
对比两条修复路线：
路线	改动范围
A. app-qemu 补 UEFI 启动（在 Context::qemu() 调 apply_dynamic_platform_qemu_boot）	框架层一处，复用已验证的 UEFI 路径
B. 让 x86 走 PVH（to_bin=false）	someboot 新增 PVH 入口 + 内存图来源 + ACPI 兜底
所以「能不能用 PVH」答案是：理论可以，但当前 someboot x86_64 是纯 UEFI 镜像，没有 PVH note、没有非 UEFI 内存图来源，做不到「只改 to_bin=false」。正确的轻量修复仍是路线 A —— 在 app-qemu 派发路径补一次 apply_dynamic_platform_qemu_boot，复用现成的 UEFI+BIN 启动。
