//! Compact token/number formatting, mirroring codex's status-line style.
//! Pure functions — unit-tested in this file.

/// Format a token count compactly: `0`, `999`, `12.35K`, `200K`, `1.2M`.
/// Uppercase suffix, 2 decimals when <10 scaled, 1 when <100, else 0.
pub fn format_tokens_compact(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let (div, suffix) = if n < 1_000_000 {
        (1_000u64, 'K')
    } else if n < 1_000_000_000 {
        (1_000_000u64, 'M')
    } else if n < 1_000_000_000_000 {
        (1_000_000_000u64, 'B')
    } else {
        (1_000_000_000_000u64, 'T')
    };
    let v = n as f64 / div as f64;
    let s = format!("{v:.2}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed}{suffix}")
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
        assert_eq!(format_tokens_compact(1_000), "1K");
        assert_eq!(format_tokens_compact(1_500), "1.5K");
        assert_eq!(format_tokens_compact(12_345), "12.35K");
        assert_eq!(format_tokens_compact(123_456), "123.46K");
        assert_eq!(format_tokens_compact(200_000), "200K");
    }

    #[test]
    fn compact_millions() {
        assert_eq!(format_tokens_compact(1_200_000), "1.2M");
        assert_eq!(format_tokens_compact(45_200_000), "45.2M");
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
}
