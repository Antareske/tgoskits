# VmX 重构与测例总结

## 范围

本文汇总 `www/vmx-refactor.md` 描述的 VmX 内存统计重构，以及当前分支中已经落地的 StarryOS 内核改动和对应测例。

## 内核改动

### 1. 引入地址空间内聚的 VmX 统计源

新增 `os/StarryOS/kernel/src/mm/vm_stat.rs`，定义 `ProcessVmStat` 作为单个地址空间的 VmX 权威数据源：

- `vss_pages: AtomicI64`
- `peak_vss_pages: AtomicU64`
- `peak_rss_pages: AtomicU64`

设计要点：

- 当前 VSS 用有符号原子计数，避免 double-unmap 或 race 把计数绕到 `u64::MAX`
- `VmPeak` / `VmHWM` 以 `fetch_max` 维护，只升不降
- `VmHWM` 仍是 Plan1 占位实现，当前与 VSS 同步，后续接真实 RSS 时只需要替换更新源

### 2. `AddrSpace` 统一维护 VmX

`os/StarryOS/kernel/src/mm/aspace/mod.rs` 为 `AddrSpace` 增加了 `vm_stat: ProcessVmStat`，并把计数更新收敛到地址空间的统一入口：

- `map()` / `map_linear()` -> `on_map()`
- `unmap()` / `unmap_metadata()` -> `on_unmap()`
- `extend_area()` -> `on_map(additional_pages)`
- `clear()` -> `on_clear()`
- `try_clone()` -> `seed_from(&parent.vm_stat)`

结果是：只要映射路径走到 `AddrSpace`，VmX 就会自动维护，不再依赖 syscall handler 手动补点。

### 3. `ProcessMemStats::collect()` 简化

`os/StarryOS/kernel/src/mm/stats.rs` 的 `collect()` 改成只接收 `&AddrSpace`：

- 取消外部传入的 peak 参数
- `VmPeak` / `VmHWM` 直接从 `aspace.vm_stat` 读取
- `format_status_vm_lines()` 继续按 Linux 风格输出 `VmPeak`、`VmSize`、`VmHWM`、`VmRSS` 等字段

### 4. 读路径切换到新接口

当前 `ProcessMemStats::collect()` 的新调用点已经统一在：

- `os/StarryOS/kernel/src/pseudofs/proc.rs`
- `os/StarryOS/kernel/src/task/stat.rs`

### 5. 模块导出整理

`os/StarryOS/kernel/src/mm/mod.rs` 注册并导出了 `vm_stat` 模块。

### 6. 语义备注

- `VmPeak`：在每次成功映射后更新峰值，`munmap` 不回落
- `VmHWM`：当前仍按 Plan1 近似为 VSS 峰值，后续接真实 RSS 后再替换
- `fork`：子进程从父进程继承高水位基线
- `execve`：通过 `clear()` 语义重置当前统计

## 测例

### 1. 保留并恢复原始 proc 测例

`test-suit/starryos/qemu-smp1/system/proc-test-proc-mem-monitor` 保持为原始 proc 监控回归，不再承载 VmX 专项逻辑。

它继续覆盖：

- `/proc/self/status` 的 `VmSize` / `VmRSS` / `VmData` / `VmStk`
- `/proc/self/statm` / `/proc/self/stat` 一致性
- `fork()` 后的 `/proc/<pid>` 读取
- `/proc/meminfo` 可解析性

### 2. 新增独立 VmX 测例

新增 `test-suit/starryos/qemu-smp1/system/proc-test-vmpeak-hwm`，只覆盖 `VmPeak` / `VmHWM`，避免与原 proc 测例重叠。

覆盖内容：

- `mremap`：扩展映射后高水位增长，`munmap` 不回落
- `mmap`：匿名大映射后高水位增长，`munmap` 不回落
- `brk`：raw `SYS_brk` 扩展堆后高水位不回落，恢复原 break

实现形式：

- `CMakeLists.txt` 安装到 `usr/bin/starry-test-suit/test-vmpeak-hwm`
- `src/main.c` 直接读取 `/proc/self/status` 里的 `VmPeak` / `VmHWM`

## 验证结果

### Linux

命令：

```bash
cmake -S test-suit/starryos/qemu-smp1/system/proc-test-vmpeak-hwm -B /tmp/opencode/proc-test-vmpeak-hwm
cmake --build /tmp/opencode/proc-test-vmpeak-hwm
/tmp/opencode/proc-test-vmpeak-hwm/test-vmpeak-hwm
```

结果：

- `TEST PASSED`
- 汇总为 `74 pass, 0 fail`

### Starry

命令：

```bash
cargo xtask starry test qemu --arch riscv64 -c qemu-smp1/system/proc-test-vmpeak-hwm
```

结果：

- `all starry qemu tests passed`
- `result: 1/1 case(s) passed`

说明：

- 该 case 起初受宿主缺少 `clang` 影响，已在本机补齐工具链后重新验证通过。

## 结论

这次改动把 VmX 从“syscall 补丁式更新”收敛为“地址空间内聚维护”，并补了一个独立的 `VmPeak` / `VmHWM` 专项测例，Linux 与 Starry 两侧都已验证通过。
