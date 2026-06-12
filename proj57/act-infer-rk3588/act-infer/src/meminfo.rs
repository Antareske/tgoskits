use std::fs;

/// Best-effort process memory footprint in kilobytes.
///
/// Primary metric: `VmHWM` (peak resident set size) from
/// `/proc/self/status`. If the running kernel's procfs does not expose
/// `VmHWM` (a real possibility on StarryOS), fall back to `VmRSS` (current
/// resident set), then to `VmPeak`/`VmSize` (virtual). Returns `None` only if
/// `/proc/self/status` is entirely unavailable or none of the fields parse.
///
/// This requires no external tooling and is emitted in the result JSON so the
/// on-board run captures memory usage even when standard monitors are missing.
pub fn peak_rss_kb() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    // Preference order: peak RSS, current RSS, peak virtual, current virtual.
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
