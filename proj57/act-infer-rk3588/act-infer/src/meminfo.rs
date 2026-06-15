use std::fs;

/// 尽力获取进程内存占用（KB）。
///
/// 优先读取 /proc/self/status 中的 VmHWM（峰值常驻集），
/// 如果当前内核没有暴露该字段（StarryOS 上是可能的），就回退到
/// VmRSS（当前常驻集），再回退到 VmPeak/VmSize（虚拟内存）。
/// 只有在 /proc/self/status 不可用或所有字段都解析失败时才返回 None。
///
/// 这样无需依赖外部工具，也能把内存指标写入结果 JSON，方便板端回收。
pub fn peak_rss_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    // 峰值常驻 -> 当前常驻 -> 峰值虚拟 -> 当前虚拟
    for key in ["VmHWM:", "VmRSS:", "VmPeak:", "VmSize:"] {
        if let Some(kb) = field_kb(&status, key) {
            return Some(kb);
        }
    }
    None
}

fn field_kb(status: &str, key: &str) -> Option<u64> {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            return rest
                .trim()
                .trim_end_matches("kB")
                .trim()
                .parse::<u64>()
                .ok();
        }
    }
    None
}
