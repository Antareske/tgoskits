# VmX 内存统计重构说明

## 背景

`/proc/[pid]/status` 的 `VmPeak` / `VmHWM` 字段原先始终返回当前 VSS，没有历史峰值追踪。
第一版修复将高水位原子计数器放在 `ProcessData`，由各 syscall handler 手动调用 `update_vm_peaks()`。
该方案存在以下问题：

- **覆盖不完整**：`mremap`、`shmat`、`execve` 等路径未更新峰值，产生实际 bug
- **不变量与变更点分离**：高水位计数器与地址空间分属不同结构，只靠约定维护，新增映射路径易遗漏
- **热路径有不必要的 O(n) 开销**：每次 `map` 后调用 `ProcessMemStats::collect()` 遍历全部 VMA 只为取一个数字

当前重构已从“syscall 补丁式更新”推进到“地址空间内聚维护”：`AddrSpace` 持有 `ProcessVmStat`，并在映射变化的统一入口增量维护 VmX 相关状态。

## 重构方案

### 新增 `mm/vm_stat.rs` — `ProcessVmStat`

所有 VmX 计数器的唯一权威数据源，嵌入 `AddrSpace`。

```
ProcessVmStat
├── vss_pages:       AtomicI64   当前虚拟内存页数（O(1) 增量维护）
├── peak_vss_pages:  AtomicU64   VmPeak 高水位（fetch_max，只升不降）
└── peak_rss_pages:   AtomicU64   VmHWM 高水位（Plan2 接入真实 RSS 时替换）
```

`AtomicI64` 有符号：防止 double-unmap 或 race 绕到 `u64::MAX`。

### 集成点：`AddrSpace` 自动维护

| 方法 | 操作 |
|------|------|
| `map()` | `on_map(pages)`：vss+，`fetch_max` 更新峰值 |
| `unmap()` / `unmap_metadata()` | `on_unmap(pages)`：vss−，峰值不变 |
| `extend_area()` | `on_map(additional_pages)` |
| `clear()` | `on_clear()`：全部归零（exec 语义） |
| `try_clone()` | `seed_from(&parent.vm_stat)`：fork 子进程继承父进程峰值 |

所有增长 VSS 的路径（mmap、brk、mremap、shmat、execve loader）全部自动覆盖，结构上不可能遗漏。

### `ProcessMemStats::collect()` 简化

签名从 `collect(aspace, peak_pages, hwm_pages)` 改为 `collect(aspace)`：
峰值直接从 `aspace.vm_stat` O(1) 读取，不再需要外部传入，也不再有 `(0, 0)` 魔法参数。

### 清理

- 删除 `ProcessData::vm_peak_pages`、`vm_hwm_pages` 字段
- 删除 `vm_peaks()`、`set_vm_peaks()`、`update_vm_peaks()` 三个方法
- 删除 `mmap.rs` / `brk.rs` 中手动 `collect + update_vm_peaks` 调用
- 删除 `clone.rs` 中手动 `set_vm_peaks` 调用（`try_clone` 内 `seed_from` 替代）
- 删除 `proc.rs` / `stat.rs` 中 `vm_peaks()` 读取调用

## 变更文件

| 文件 | 变更类型 |
|------|---------|
| `mm/vm_stat.rs` | 新增 |
| `mm/mod.rs` | 注册新模块 |
| `mm/aspace/mod.rs` | 嵌入 `ProcessVmStat`，接入各变更点 |
| `mm/stats.rs` | `collect()` 去掉两个峰值参数 |
| `task/mod.rs` | 删除旧字段和三个方法 |
| `syscall/mm/mmap.rs` | 删除手动峰值更新逻辑 |
| `syscall/mm/brk.rs` | 删除手动峰值更新逻辑 |
| `syscall/task/clone.rs` | 删除手动 `set_vm_peaks` |
| `pseudofs/proc.rs` | 更新 `collect()` 调用签名 |
| `task/stat.rs` | 更新 `collect()` 调用签名 |

## 验证

- `cargo xtask clippy --package starry-kernel`：15/15 检查通过，0 警告，0 错误
- 尚未完成面向 `/proc/[pid]/status` 的 QEMU 测试验证
- 尚未添加专门覆盖 `VmPeak` / `VmHWM` 行为的 proc 测例

已尝试运行 `cargo xtask starry test qemu --arch riscv64 -c qemu-smp1/system/proc-test-proc-mem-monitor`，但当前环境缺少 `clang`，测试未能进入有效验证阶段。

## TODO

- 补充相关 proc 测例，覆盖 `VmPeak` / `VmHWM` 在 `mmap`、`brk`、`mremap`、`munmap` 等路径下的增长和不回落语义
- 安装或切换到具备完整工具链的环境后，重新运行 `qemu-smp1/system/proc-test-proc-mem-monitor` 及新增 proc 测例
- 对照 Linux 源码中 `mm_struct` / `hiwater_vm` / `hiwater_rss` 等实现，后续将当前 StarryOS 版本演进为更成熟、语义更一致的实现
- 在真实 RSS 统计落地后，将 `VmHWM` 从当前占位式近似更新切换为基于真实 RSS 的高水位维护

## Plan2 扩展路径

当真实 RSS 统计落地时，只需：
1. 在 `ProcessVmStat` 中添加 `rss_pages: AtomicI64`
2. 在 page-fault / reclaim 路径更新它
3. 将 `on_map()` 中 `peak_rss_pages.fetch_max` 的参数从 VSS 改为真实 RSS

其余消费方（`collect()`、`format_status_vm_lines()`）无需改动。
