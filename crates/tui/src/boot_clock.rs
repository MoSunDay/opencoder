//! Process-lifetime boot clock for TUI cold-start instrumentation.
//!
//! `mark` records the boot instant exactly once (first call wins);
//! `note_first_frame` logs the mark-to-first-frame latency exactly once. Both
//! are safe no-ops when called out of order or repeatedly, so call sites
//! (bootstrap entry, the frame renderer) never need guarding.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

/// Sentinel stored in the mark cell until [`mark`] fires (unix millis are
/// always positive, so a negative value can never be a real mark).
const UNMARKED_MS: i64 = -1;

/// Frames slower than this many millis get a warn-level line on top of info.
const SLOW_FRAME_MS: u64 = 1000;

static BOOT_MARK_MS: AtomicI64 = AtomicI64::new(UNMARKED_MS);
static FIRST_FRAME_NOTED: AtomicBool = AtomicBool::new(false);

/// Pure decision for the first-frame log line.
///
/// `Some(ms)` = elapsed millis between mark and frame (saturating at zero so a
/// backwards wall clock cannot produce a negative field). `None` = do not log:
/// either the clock was never marked or the first frame was already noted.
fn frame_log_ms(start_ms: i64, now_ms: i64, logged: bool) -> Option<u64> {
    if logged || start_ms < 0 {
        return None;
    }
    Some((now_ms - start_ms).max(0) as u64)
}

/// Threshold predicate kept pure so the warn path is testable without a
/// tracing subscriber.
fn is_slow_frame(ms: u64) -> bool {
    ms > SLOW_FRAME_MS
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Pure gate for a wall-clock read becoming the boot mark.
///
/// `unix_ms` falls back to `0` when the clock read fails (pre-epoch or
/// unset system time). A `0` (or negative) reading must never be stored:
/// the [`UNMARKED_MS`] sentinel would silently become a "marked at epoch"
/// value and `note_first_frame` would log an absurd first-frame latency.
/// `None` keeps the sentinel in place, leaving `note_first_frame` a no-op.
fn mark_candidate(now_ms: i64) -> Option<i64> {
    (now_ms > 0).then_some(now_ms)
}

/// Record the boot start instant once. Repeated calls are harmless no-ops.
pub fn mark() {
    // A clock failure (`mark_candidate` -> None) must not write the mark:
    // the UNMARKED sentinel stays and first-frame logging stays off.
    if let Some(now) = mark_candidate(unix_ms()) {
        let _ =
            BOOT_MARK_MS.compare_exchange(UNMARKED_MS, now, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// Log the mark-to-first-frame latency, exactly once.
///
/// No-op unless [`mark`] ran earlier; the once-guard makes every later frame
/// (and any race between two frames) log nothing.
pub fn note_first_frame() {
    let start_ms = BOOT_MARK_MS.load(Ordering::Relaxed);
    let Some(ms) = frame_log_ms(
        start_ms,
        unix_ms(),
        FIRST_FRAME_NOTED.load(Ordering::Relaxed),
    ) else {
        return;
    };
    // First writer wins: a racing frame must not double-log.
    if FIRST_FRAME_NOTED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    tracing::info!("first frame in {ms}ms");
    if is_slow_frame(ms) {
        tracing::warn!("first frame in {ms}ms exceeds {SLOW_FRAME_MS}ms budget");
    }
}

#[cfg(test)]
mod tests {
    use super::{frame_log_ms, is_slow_frame, mark_candidate, UNMARKED_MS};

    /// `note_first_frame` before `mark` must be a safe no-op: the unmarked
    /// sentinel forces the computation to `None`, so nothing is logged.
    #[test]
    fn unmarked_start_is_a_no_op() {
        assert_eq!(frame_log_ms(UNMARKED_MS, 5_000, false), None);
    }

    /// The first frame logs exactly once: after the once-guard flips, later
    /// frames compute to `None`.
    #[test]
    fn first_frame_is_logged_only_once() {
        assert_eq!(frame_log_ms(1_000, 1_250, false), Some(250));
        assert_eq!(frame_log_ms(1_000, 1_300, true), None);
    }

    /// Elapsed millis are measured from the mark and saturate at zero, so a
    /// backwards wall clock cannot yield a negative duration.
    #[test]
    fn elapsed_saturates_at_zero() {
        assert_eq!(frame_log_ms(2_000, 1_500, false), Some(0));
        assert_eq!(frame_log_ms(1_000, 1_000, false), Some(0));
    }

    /// Only frames strictly over the budget take the warn path.
    #[test]
    fn warn_fires_only_past_threshold() {
        assert!(is_slow_frame(1_001));
        assert!(!is_slow_frame(1_000));
        assert!(!is_slow_frame(5));
    }

    /// Clock-failure regression: `unix_ms` falls back to `0` when the wall
    /// clock cannot be read. `mark` must refuse such a reading (the
    /// `UNMARKED_MS` sentinel stays), otherwise the mark cell holds "booted at
    /// the epoch" and the first frame logs an absurd latency.
    #[test]
    fn mark_rejects_nonpositive_clock_readings() {
        assert_eq!(mark_candidate(0), None, "0 is the clock-failure fallback");
        assert_eq!(mark_candidate(-1), None, "pre-epoch readings are bogus");
        assert_eq!(mark_candidate(i64::MIN), None);
        assert_eq!(mark_candidate(1), Some(1), "epoch+1ms is a legal mark");
        assert_eq!(mark_candidate(1_756_540_800_000), Some(1_756_540_800_000));
    }

    /// Why the gate matters: if `0` ever leaked into the mark cell,
    /// `frame_log_ms` would happily report an absurd elapsed value (only
    /// negative start values are hidden). The `mark_candidate` gate is what
    /// keeps the cell at the sentinel on a clock failure.
    #[test]
    fn stored_zero_would_produce_an_absurd_latency() {
        assert_eq!(frame_log_ms(0, i64::MAX, false), Some(i64::MAX as u64));
    }
}
