//! Compact token/number formatting, mirroring codex's status-line style.
//! Pure functions — unit-tested in this file.

/// Format a token count compactly: `0`, `999`, `12.35k`, `200k`, `1.2m`.
/// Lowercase suffix, 2 decimals when <10 scaled, 1 when <100, else 0.
pub fn format_tokens_compact(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let (div, suffix) = if n < 1_000_000 {
        (1_000u64, 'k')
    } else if n < 1_000_000_000 {
        (1_000_000u64, 'm')
    } else if n < 1_000_000_000_000 {
        (1_000_000_000u64, 'b')
    } else {
        (1_000_000_000_000u64, 't')
    };
    let v = n as f64 / div as f64;
    let s = format!("{v:.2}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed}{suffix}")
}

/// Session-lifetime token cost for the body's bottom-left `[tok cost]` corner:
/// `0` when empty, else millions with three decimals (`m` = 1,000,000 tokens),
/// floored at `0.001m`. Pure function.
pub fn format_tokens_cost_m(total: u64) -> String {
    if total == 0 {
        return "0".to_string();
    }
    let m = total as f64 / 1_000_000.0;
    format!("{:.3}m", m.max(0.001))
}

/// Context-window usage percent. Mirrors codex's baseline-offset math: subtract
/// a baseline from both used and window so small sessions read ~0% rather than
/// a misleading fraction of the full window. Clamps to [0, 100].
pub fn context_percent(used: u64, window: u64, baseline: u64) -> u8 {
    let eff_window = window.saturating_sub(baseline);
    let eff_used = used.saturating_sub(baseline);
    if eff_window == 0 {
        return 0;
    }
    let pct = (eff_used as f64 / eff_window as f64) * 100.0;
    pct.round().clamp(0.0, 100.0) as u8
}

/// Format a run-duration in ms as a short label: `42s`, `3m5s`, `1h5m30s`.
/// Always shows the seconds component; largest unit is hours.
pub fn format_run_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m}m{s}s")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_small_is_plain() {
        assert_eq!(format_tokens_compact(0), "0");
        assert_eq!(format_tokens_compact(999), "999");
    }

    #[test]
    fn compact_thousands() {
        assert_eq!(format_tokens_compact(1_000), "1k");
        assert_eq!(format_tokens_compact(1_500), "1.5k");
        assert_eq!(format_tokens_compact(12_345), "12.35k");
        assert_eq!(format_tokens_compact(123_456), "123.46k");
        assert_eq!(format_tokens_compact(200_000), "200k");
    }

    #[test]
    fn compact_millions() {
        assert_eq!(format_tokens_compact(1_200_000), "1.2m");
        assert_eq!(format_tokens_compact(45_200_000), "45.2m");
    }

    #[test]
    fn context_percent_uses_baseline_and_clamps() {
        // used < baseline → 0%
        assert_eq!(context_percent(5_000, 200_000, 12_000), 0);
        // (50000-12000)/(200000-12000) = 38000/188000 ≈ 20%
        assert_eq!(context_percent(50_000, 200_000, 12_000), 20);
        // overflow clamps to 100
        assert_eq!(context_percent(500_000, 200_000, 12_000), 100);
        // zero window → 0 (no panic)
        assert_eq!(context_percent(100, 0, 0), 0);
    }

    #[test]
    fn tok_cost_zero_and_floor_at_one_thousandth_million() {
        assert_eq!(format_tokens_cost_m(0), "0");
        assert_eq!(format_tokens_cost_m(1), "0.001m");
        assert_eq!(format_tokens_cost_m(50_000), "0.050m");
        assert_eq!(format_tokens_cost_m(100_000), "0.100m");
    }

    #[test]
    fn tok_cost_scales_in_millions() {
        assert_eq!(format_tokens_cost_m(150_000), "0.150m");
        assert_eq!(format_tokens_cost_m(300_000), "0.300m");
        assert_eq!(format_tokens_cost_m(1_234_567), "1.235m");
        assert_eq!(format_tokens_cost_m(5_000_000), "5.000m");
        assert_eq!(format_tokens_cost_m(12_600_000), "12.600m");
        assert_eq!(format_tokens_cost_m(999_999), "1.000m");
    }

    #[test]
    fn run_duration_formats_correctly() {
        assert_eq!(format_run_duration(0), "0s");
        assert_eq!(format_run_duration(999), "0s");
        assert_eq!(format_run_duration(1000), "1s");
        assert_eq!(format_run_duration(59000), "59s");
        assert_eq!(format_run_duration(60000), "1m0s");
        assert_eq!(format_run_duration(119000), "1m59s");
        assert_eq!(format_run_duration(120000), "2m0s");
        assert_eq!(format_run_duration(3599000), "59m59s");
        assert_eq!(format_run_duration(3600000), "1h0m0s");
        assert_eq!(format_run_duration(3900000), "1h5m0s");
        assert_eq!(format_run_duration(7384000), "2h3m4s");
    }
}
