//! CPU capacity is a quota, not host utilization or a synthetic load average.
use std::path::Path;

pub fn quota(text: &str) -> Option<f64> {
    let mut words = text.split_whitespace();
    let limit: f64 = words.next()?.parse().ok()?;
    let period: f64 = words.next()?.parse().ok()?;
    let value = limit / period;
    (limit > 0.0 && period > 0.0 && value.is_finite()).then_some(value)
}

pub fn capacity() -> f64 {
    let mut capacity = std::thread::available_parallelism()
        .map(|n| n.get() as f64)
        .unwrap_or(1.0);
    // Walk ancestors: a parent cgroup may impose a tighter limit than the leaf.
    if let Ok(cgroups) = std::fs::read_to_string("/proc/self/cgroup") {
        if let Some(relative) = cgroups.lines().find_map(|l| l.strip_prefix("0::")) {
            let root = Path::new("/sys/fs/cgroup");
            let mut current = root.join(relative.trim_start_matches('/'));
            while current.starts_with(root) {
                if let Ok(text) = std::fs::read_to_string(current.join("cpu.max")) {
                    if let Some(limit) = quota(&text) {
                        capacity = capacity.min(limit);
                    }
                }
                if !current.pop() {
                    break;
                }
            }
        }
    }
    // cgroup v1 installations commonly mount the CPU controller here.
    if let (Ok(limit), Ok(period)) = (
        std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_quota_us"),
        std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_period_us"),
    ) {
        if let Some(value) = quota(&format!("{} {}", limit.trim(), period.trim())) {
            capacity = capacity.min(value);
        }
    }
    capacity
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_and_unlimited_cpu() {
        assert_eq!(quota("150000 100000"), Some(1.5));
        assert_eq!(quota("max 100000"), None);
        assert_eq!(quota("-1 100000"), None);
        assert_eq!(quota("100 0"), None);
        assert_eq!(quota("NaN 100"), None);
        assert!(capacity().is_finite() && capacity() > 0.0);
    }
}
