//! Tool-failure threshold guard with exponential backoff.
//!
//! Tracks consecutive failures per tool name across turns within a single
//! `run_loop`. When a tool name hits the configured threshold, the turn is
//! aborted. Between failures, an exponential backoff delay is applied.

use std::collections::HashMap;
use std::time::Duration;

use opencoder_core::ToolGuardConfig;

/// Maps tool name to consecutive failure count.
pub type FailureMap = HashMap<String, u32>;

/// Record a single tool result into `map`.
///
/// On success the counter for `name` is reset to zero. On failure it is
/// incremented. Returns `(tripped, backoff)` where `tripped` is true when the
/// threshold was reached and `backoff` is the delay to apply.
pub fn record(
    map: &mut FailureMap,
    name: &str,
    is_error: bool,
    cfg: &ToolGuardConfig,
) -> (bool, Duration) {
    if cfg.max_consecutive_failures == 0 {
        return (false, Duration::ZERO);
    }
    if !is_error {
        map.remove(name);
        return (false, Duration::ZERO);
    }
    let count = map.entry(name.to_string()).or_insert(0);
    *count += 1;
    let tripped = *count >= cfg.max_consecutive_failures;
    (tripped, backoff(*count, cfg))
}

/// Exponential backoff for failure number `count` (1-based).
fn backoff(count: u32, cfg: &ToolGuardConfig) -> Duration {
    if count == 0 {
        return Duration::ZERO;
    }
    let exp = (count - 1).min(20);
    let ms = cfg
        .backoff_base_ms
        .saturating_mul(1u64 << exp)
        .min(cfg.backoff_max_ms);
    Duration::from_millis(ms)
}

/// The worst (name, count) pair in the map, for diagnostic messages.
pub fn worst(map: &FailureMap) -> Option<(&str, u32)> {
    map.iter()
        .max_by_key(|(_, &c)| c)
        .map(|(n, &c)| (n.as_str(), c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ToolGuardConfig {
        ToolGuardConfig {
            max_consecutive_failures: 3,
            backoff_base_ms: 200,
            backoff_max_ms: 2000,
        }
    }

    #[test]
    fn success_resets_counter() {
        let mut m = FailureMap::new();
        let (t, _) = record(&mut m, "bash", true, &cfg());
        assert!(!t);
        assert_eq!(m.get("bash"), Some(&1));

        let (t, _) = record(&mut m, "bash", false, &cfg());
        assert!(!t);
        assert!(!m.contains_key("bash"));
    }

    #[test]
    fn threshold_trips_exactly_at_limit() {
        let mut m = FailureMap::new();
        let c = cfg();
        let (t1, _) = record(&mut m, "bash", true, &c);
        assert!(!t1);
        let (t2, _) = record(&mut m, "bash", true, &c);
        assert!(!t2);
        let (t3, _) = record(&mut m, "bash", true, &c);
        assert!(t3);
    }

    #[test]
    fn backoff_exponential_and_capped() {
        let mut m = FailureMap::new();
        let c = cfg();
        let (_, d1) = record(&mut m, "t", true, &c);
        assert_eq!(d1.as_millis(), 200);
        let (_, d2) = record(&mut m, "t", true, &c);
        assert_eq!(d2.as_millis(), 400);
        let (_, d3) = record(&mut m, "t", true, &c);
        assert_eq!(d3.as_millis(), 800);
        let (_, d4) = record(&mut m, "t", true, &c);
        assert_eq!(d4.as_millis(), 1600);
        let (_, d5) = record(&mut m, "t", true, &c);
        assert_eq!(d5.as_millis(), 2000);
        let (_, d6) = record(&mut m, "t", true, &c);
        assert_eq!(d6.as_millis(), 2000);
    }

    #[test]
    fn independent_tools_tracked_separately() {
        let mut m = FailureMap::new();
        record(&mut m, "bash", true, &cfg());
        record(&mut m, "read", true, &cfg());
        assert_eq!(m.get("bash"), Some(&1));
        assert_eq!(m.get("read"), Some(&1));
    }

    #[test]
    fn success_between_failures_resets() {
        let mut m = FailureMap::new();
        let c = cfg();
        record(&mut m, "bash", true, &c);
        record(&mut m, "bash", true, &c);
        record(&mut m, "bash", false, &c);
        let (t, _) = record(&mut m, "bash", true, &c);
        assert!(!t);
        assert_eq!(m.get("bash"), Some(&1));
    }

    #[test]
    fn zero_threshold_disables_guard() {
        let mut m = FailureMap::new();
        let c = ToolGuardConfig {
            max_consecutive_failures: 0,
            backoff_base_ms: 200,
            backoff_max_ms: 2000,
        };
        let (t, d) = record(&mut m, "x", true, &c);
        assert!(!t);
        assert!(d.is_zero());
    }

    #[test]
    fn worst_returns_highest_count() {
        let mut m = FailureMap::new();
        m.insert("a".into(), 1);
        m.insert("b".into(), 3);
        m.insert("c".into(), 2);
        assert_eq!(worst(&m), Some(("b", 3)));
    }

    #[test]
    fn worst_empty_is_none() {
        let m = FailureMap::new();
        assert_eq!(worst(&m), None);
    }
}
