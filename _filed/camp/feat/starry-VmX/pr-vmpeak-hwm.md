# StarryOS /proc VmPeak 与 VmHWM 实现

## 背景

StarryOS 的 `/proc/[pid]/status` 已经输出 Vm* 字段，但 `VmPeak` 和 `VmHWM` 直接跟随当前 `VmSize` / `VmRSS` 结果，进程释放映射后峰值也会下降，不符合 Linux 中高水位线单调不下降的语义。

这会影响依赖 `/proc/self/status` 判断进程历史最大虚拟内存和历史最大驻留内存的程序，也让 `mmap`、`mremap`、`brk` 等内存变化路径缺少可回归验证。

## 方案

在 StarryOS 地址空间 `AddrSpace` 内新增统一的进程虚拟内存统计对象 `ProcessVmStat`，由地址空间的映射生命周期自动维护 VmX 计数，而不是让 syscall 处理路径手动维护。

核心思路如下：

1. 当前虚拟内存页数在成功 `map` / `extend_area` 后增加，在 `unmap` / `unmap_metadata` 后按实际移除的映射范围减少，在 `clear` 时重置。
2. `VmPeak` 使用 `peak_vss_pages` 记录历史最大虚拟地址空间页数，只在映射增长路径中通过高水位更新，不随 unmap 降低。
3. `VmHWM` 当前阶段仍沿用 Plan1 语义，跟随虚拟内存统计作为 RSS 上界；后续真实 RSS 统计落地后可切换为真实驻留集高水位。
4. `fork` / `clone` 复制地址空间时，通过 `seed_from` 让子进程继承父进程当前地址空间大小作为初始统计基线，并保留合理的峰值语义。
5. `/proc/[pid]/status` 渲染时继续通过 VMA 遍历获取当前 VmSize / VmRSS / VmData / VmStk / VmExe，同时从 `AddrSpace::vm_stat` O(1) 读取 `VmPeak` 和 `VmHWM`。

## 改动

最后两次提交包含以下改动：

1. `fix(starry-kernel): track proc vm watermarks`

   引入 `os/StarryOS/kernel/src/mm/vm_stat.rs`，新增 `ProcessVmStat`，使用原子计数维护当前 VSS、峰值 VSS 和峰值 RSS。`vss_pages` 使用有符号原子值，避免异常重复 unmap 时出现无符号下溢。

2. `AddrSpace` 集成 VmX 统计

   `os/StarryOS/kernel/src/mm/aspace/mod.rs` 在 `map`、`extend_area`、`unmap`、`unmap_metadata`、`clear`、`try_clone` 中维护 `vm_stat`。unmap 路径先按待删除范围和现有 VMA 的交集计算实际移除页数，再更新当前 VSS，避免按请求长度错误扣减。

3. `/proc` 内存状态输出修正

   `os/StarryOS/kernel/src/mm/stats.rs` 在 `ProcessMemStats` 中新增 `peak_pages` 和 `hwm_pages`，`format_status_vm_lines` 改为使用这两个高水位字段输出 `VmPeak` / `VmHWM`，并补充格式化单元测试覆盖。

4. mmap / brk 锁生命周期调整

   `os/StarryOS/kernel/src/syscall/mm/brk.rs` 和 `os/StarryOS/kernel/src/syscall/mm/mmap.rs` 在映射成功后显式释放地址空间锁，避免后续流程继续持有锁导致不必要的锁生命周期延长。

5. 新增 StarryOS 回归测例

   新增 `test-suit/starryos/qemu-smp1/system/proc-test-vmpeak-hwm`，测试程序读取 `/proc/self/status` 中的 `VmPeak` 和 `VmHWM`，覆盖 `mmap`、`mremap`、`brk` 三类增长路径，并验证释放映射后 `VmPeak` / `VmHWM` 不下降。

## 本地验证

已完成以下本地验证：

1. Linux 下该 case 测试通过。
2. 四个架构的该 case 测例通过。
3. 一次 `cargo xtask starry test qemu --target riscv64gc-unknown-none-elf` 通过。
