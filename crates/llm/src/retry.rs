//! Pure retry policy for the LLM streaming client.
//!
//! Every retry-vs-fail-vs-done decision (both the pre-stream connection loop
//! and the mid-stream interruption loop) is a pure function of an abstract
//! outcome + attempt counter, kept free of I/O so the boundary logic — the part
//! most prone to off-by-one errors — is exhaustively unit-testable.
//!
//! Two retry budgets exist, kept deliberately separate:
//! - [`MAX_ATTEMPTS`] — pre-stream connection/HTTP-status retries (handled by
//!   `connect_with_retry` in `client.rs`). Fires before any byte is streamed.
//! - [`MAX_STREAM_ATTEMPTS`] — mid-stream retries after the connection is up and
//!   bytes have started flowing (chunk read errors, truncated streams, idle
//!   stalls). See `run_stream` in `client.rs`.
//!
//! Both loops back off exponentially (0.5 s doubling per attempt); no single
//! wait — computed backoff or server `Retry-After` hint — exceeds
//! [`BACKOFF_MAX_MS`] (5 minutes).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Total pre-stream connection attempts (1 initial + 9 retries).
pub const MAX_ATTEMPTS: u8 = 10;

/// Total mid-stream attempts (1 initial + 2 retries). Capped low because each
/// retry discards the partial response and regenerates it from scratch, so the
/// worst-case token cost is bounded at 3× a single turn.
pub const MAX_STREAM_ATTEMPTS: u8 = 3;

/// Base backoff in ms; actual delay is `BASE_BACKOFF_MS * 2^(attempt-1)` plus
/// up to 250 ms jitter, giving roughly 0.5/1/2/4/8/16/32/64/128/256 s between
/// attempts 1-10, clamped to [`BACKOFF_MAX_MS`].
const BASE_BACKOFF_MS: u64 = 500;

/// Hard ceiling on a single wait between attempts (5 minutes). Applies both to
/// the computed exponential backoff and to any server `Retry-After` hint, so no
/// single sleep can stall a session beyond this bound.
pub const BACKOFF_MAX_MS: u64 = 300_000;

/// Whether an HTTP status is transient enough to warrant a retry. Network/send
/// errors (no status) are always retried; only these status codes qualify.
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Classification of a single send attempt's outcome, abstracted away from
/// `reqwest` so the retry decision can be unit-tested without HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// 2xx — the request succeeded; stop and consume the response.
    Success,
    /// A transient failure worth retrying (whitelisted status, or a network/
    /// transport error with no status at all).
    RetryableError,
    /// A permanent failure (4xx other than the whitelist) — fail immediately.
    NonRetryableError,
}

impl AttemptOutcome {
    /// Classify an HTTP response status into an attempt outcome.
    pub fn from_status(status: reqwest::StatusCode) -> Self {
        if status.is_success() {
            Self::Success
        } else if is_retryable_status(status) {
            Self::RetryableError
        } else {
            Self::NonRetryableError
        }
    }
}

/// What the retry loop should do after observing an attempt's outcome, given
/// the current 1-based `attempt` number and the `max` attempts allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Request succeeded — stop and return the response.
    Done,
    /// Transient failure, attempts remaining — emit `Retrying` and back off.
    Retry,
    /// Permanent failure OR retries exhausted — stop with an error.
    Fail,
}

/// Pure retry policy with no I/O, so the loop's boundary logic (the part prone
/// to off-by-one errors) is exhaustively unit-testable. Both retry loops
/// delegate every retry-vs-fail-vs-done decision here.
pub fn retry_decision(outcome: AttemptOutcome, attempt: u8, max: u8) -> RetryDecision {
    match outcome {
        AttemptOutcome::Success => RetryDecision::Done,
        AttemptOutcome::NonRetryableError => RetryDecision::Fail,
        AttemptOutcome::RetryableError => {
            if attempt >= max {
                RetryDecision::Fail
            } else {
                RetryDecision::Retry
            }
        }
    }
}

/// Exponential backoff delay (ms) for the given 1-based `attempt`, BEFORE
/// jitter: `BASE_BACKOFF_MS * 2^(attempt-1)` → 500/1000/2000/.../256000 ms,
/// clamped to [`BACKOFF_MAX_MS`]. Extracted as a pure function so the growth
/// curve and the ceiling are unit-testable.
pub fn backoff_millis(attempt: u8) -> u64 {
    BASE_BACKOFF_MS
        .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1) as u32))
        .min(BACKOFF_MAX_MS)
}

/// Exponential backoff for the given 1-based `attempt`, with up to 250 ms of
/// jitter derived from the wall clock (no `rand` dependency). Jitter avoids
/// synchronized retry bursts when many clients share a flaky endpoint.
pub async fn backoff_delay(attempt: u8) {
    tokio::time::sleep(backoff_duration(attempt)).await;
}

/// The (jittered) backoff [`Duration`] that [`backoff_delay`] would sleep for,
/// without sleeping. Exposed so callers can combine it with server-provided
/// hints (e.g. an HTTP `Retry-After` header) before deciding how long to wait.
pub fn backoff_duration(attempt: u8) -> Duration {
    let exp = backoff_millis(attempt);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() as u64) % 250)
        .unwrap_or(0);
    Duration::from_millis(exp.saturating_add(jitter).min(BACKOFF_MAX_MS))
}

/// Upper bound on how much of a server-sent `Retry-After` hint (seconds, or an
/// HTTP-date) we are willing to honor. A hostile or misbehaving server replying
/// `429` + `Retry-After: 86400` must not stall a session for a full day — the
/// retry loop would sit in `sleep` with nothing but channel-close to break it.
/// Aligned with [`BACKOFF_MAX_MS`] so a single wait never exceeds 5 minutes.
pub const RETRY_AFTER_MAX_SECS: u64 = 300;

/// Pure function deciding the delay to sleep before the next attempt when the
/// server supplied a `Retry-After` hint. The hint is floored at 1 s, capped at
/// [`RETRY_AFTER_MAX_SECS`], then combined with the locally computed backoff
/// by taking the maximum — so we never wait less than either side wants, and
/// never more than the cap above the backoff.
pub fn retry_delay(retry_after_secs: Option<u64>, computed: Duration) -> Duration {
    match retry_after_secs {
        Some(secs) => computed.max(Duration::from_secs(secs.clamp(1, RETRY_AFTER_MAX_SECS))),
        None => computed,
    }
}

/// Classification of a mid-stream interruption. All three indicate a transient
/// upstream/network fault rather than a logic error in the request, so all are
/// retryable up to [`MAX_STREAM_ATTEMPTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamInterruption {
    /// A chunk read failed mid-stream (connection reset, transport error, or a
    /// per-read timeout from the underlying HTTP client).
    ChunkError,
    /// The stream ended cleanly but carried no valid `finish_reason` — the
    /// response was truncated. Retry, then report an error if the budget is exhausted.
    Truncated,
    /// No decoded SSE event arrived within the idle window. Catches an upstream
    /// that keeps the connection alive with keep-alive heartbeats but delivers
    /// no content.
    IdleTimeout,
}

impl std::fmt::Display for StreamInterruption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ChunkError => "chunk read error",
            Self::Truncated => "truncated stream",
            Self::IdleTimeout => "idle timeout",
        })
    }
}

/// Whether a mid-stream interruption is worth retrying. Every current class is
/// retryable; this pure classifier exists so the decision is testable in
/// isolation and so future non-retryable reasons can opt out here.
pub fn should_retry_stream_interruption(reason: StreamInterruption) -> bool {
    matches!(
        reason,
        StreamInterruption::ChunkError
            | StreamInterruption::Truncated
            | StreamInterruption::IdleTimeout
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_status_whitelist() {
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert!(
                is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()),
                "{code} should be retryable"
            );
        }
    }

    #[test]
    fn non_retryable_status_fails_fast() {
        for code in [400, 401, 403, 404, 422] {
            assert!(
                !is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()),
                "{code} should fail fast"
            );
        }
        assert!(!is_retryable_status(reqwest::StatusCode::OK));
    }

    /// The backoff curve doubles each attempt: 0.5/1/2/4/8/16/32/64/128/256 s
    /// for attempts 1-10 (the full [`MAX_ATTEMPTS`] budget).
    #[test]
    fn backoff_millis_doubles_each_attempt() {
        let expected = [
            500u64, 1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000, 128_000, 256_000,
        ];
        for (idx, ms) in expected.iter().enumerate() {
            assert_eq!(backoff_millis(idx as u8 + 1), *ms, "attempt {}", idx + 1);
        }
        assert_eq!(MAX_ATTEMPTS, 10);
        assert_eq!(MAX_STREAM_ATTEMPTS, 3);
    }

    /// A single wait never exceeds [`BACKOFF_MAX_MS`] (5 minutes), even far
    /// beyond the budget or at the extreme of the attempt counter.
    #[test]
    fn backoff_is_capped_at_five_minutes() {
        assert_eq!(BACKOFF_MAX_MS, 300_000);
        for attempt in [11u8, 12, 20, u8::MAX] {
            assert_eq!(
                backoff_millis(attempt),
                BACKOFF_MAX_MS,
                "attempt {attempt} must be clamped"
            );
            assert!(
                backoff_duration(attempt) <= Duration::from_millis(BACKOFF_MAX_MS),
                "attempt {attempt}: jitter pushed the wait past the ceiling"
            );
        }
    }

    #[test]
    fn backoff_duration_within_millis_plus_jitter() {
        // backoff_duration must be the pure backoff plus at most 250 ms of
        // jitter, so a caller can safely combine it with a Retry-After hint via
        // `Duration::max` without surprising bounds.
        for attempt in 1..=10u8 {
            let lo = backoff_millis(attempt);
            let hi = lo + 250;
            let d = backoff_duration(attempt);
            assert!(
                d >= Duration::from_millis(lo) && d <= Duration::from_millis(hi),
                "attempt {attempt}: backoff_duration {d:?} not within [{lo}, {hi}] ms"
            );
        }
    }

    // ---- retry_delay: server `Retry-After` honored, but bounded ----

    /// Regression: `Retry-After: 86400` used to sleep a full day; it must now
    /// be capped at [`RETRY_AFTER_MAX_SECS`].
    #[test]
    fn retry_delay_caps_server_hint_at_max() {
        assert_eq!(RETRY_AFTER_MAX_SECS, 300);
        assert_eq!(
            retry_delay(Some(86_400), Duration::from_secs(5)),
            Duration::from_secs(RETRY_AFTER_MAX_SECS)
        );
        // Exactly at the cap is honored as-is.
        assert_eq!(
            retry_delay(Some(300), Duration::from_secs(5)),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn retry_delay_floors_zero_and_one_to_one_second() {
        assert_eq!(
            retry_delay(Some(0), Duration::from_millis(500)),
            Duration::from_secs(1)
        );
        assert_eq!(
            retry_delay(Some(1), Duration::from_millis(500)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn retry_delay_without_hint_returns_computed_unchanged() {
        let computed = Duration::from_millis(750);
        assert_eq!(retry_delay(None, computed), computed);
    }

    #[test]
    fn retry_delay_takes_max_of_capped_hint_and_backoff() {
        // Hint above the (small) backoff wins...
        assert_eq!(
            retry_delay(Some(30), Duration::from_secs(5)),
            Duration::from_secs(30)
        );
        // ...and a backoff larger than the hint is never shortened by it.
        assert_eq!(
            retry_delay(Some(30), Duration::from_secs(60)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn attempt_outcome_classifies_status() {
        use reqwest::StatusCode;
        assert_eq!(
            AttemptOutcome::from_status(StatusCode::OK),
            AttemptOutcome::Success
        );
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert_eq!(
                AttemptOutcome::from_status(StatusCode::from_u16(code).unwrap()),
                AttemptOutcome::RetryableError,
                "{code} should classify as retryable"
            );
        }
        for code in [400, 401, 403, 404, 422] {
            assert_eq!(
                AttemptOutcome::from_status(StatusCode::from_u16(code).unwrap()),
                AttemptOutcome::NonRetryableError,
                "{code} should classify as non-retryable"
            );
        }
    }

    /// Success always stops immediately, regardless of attempt number.
    #[test]
    fn retry_decision_success_stops() {
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 1, MAX_ATTEMPTS),
            RetryDecision::Done
        );
        assert_eq!(
            retry_decision(AttemptOutcome::Success, MAX_ATTEMPTS, MAX_ATTEMPTS),
            RetryDecision::Done
        );
    }

    /// A non-retryable error fails FAST on every attempt — never retries.
    #[test]
    fn retry_decision_non_retryable_fails_fast() {
        for attempt in 1..=MAX_ATTEMPTS {
            assert_eq!(
                retry_decision(AttemptOutcome::NonRetryableError, attempt, MAX_ATTEMPTS),
                RetryDecision::Fail,
                "non-retryable must fail fast at attempt {attempt}"
            );
        }
    }

    /// A retryable error retries while attempts remain and FAILS exactly when
    /// attempt == max (no sixth attempt). This is the off-by-one canary.
    #[test]
    fn retry_decision_retryable_retries_then_fails_at_max() {
        for attempt in 1..MAX_ATTEMPTS {
            assert_eq!(
                retry_decision(AttemptOutcome::RetryableError, attempt, MAX_ATTEMPTS),
                RetryDecision::Retry,
                "attempt {attempt} (< max={MAX_ATTEMPTS}) should retry"
            );
        }
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, MAX_ATTEMPTS, MAX_ATTEMPTS),
            RetryDecision::Fail,
            "attempt == max must fail (no attempt beyond max)"
        );
    }

    /// Replay a full retry-then-recover sequence through the policy: 2
    /// retryable failures then success.
    #[test]
    fn retry_decision_sequence_recover_on_third_attempt() {
        let max = MAX_ATTEMPTS;
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 1, max),
            RetryDecision::Retry
        );
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 2, max),
            RetryDecision::Retry
        );
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 3, max),
            RetryDecision::Done
        );
    }

    /// Every mid-stream interruption class is retryable.
    #[test]
    fn all_stream_interruptions_are_retryable() {
        for reason in [
            StreamInterruption::ChunkError,
            StreamInterruption::Truncated,
            StreamInterruption::IdleTimeout,
        ] {
            assert!(
                should_retry_stream_interruption(reason),
                "{reason:?} should be retryable"
            );
        }
    }
}
