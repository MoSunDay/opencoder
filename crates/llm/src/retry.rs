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

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Total pre-stream connection attempts (1 initial + 4 retries).
pub const MAX_ATTEMPTS: u8 = 5;

/// Total mid-stream attempts (1 initial + 2 retries). Capped low because each
/// retry discards the partial response and regenerates it from scratch, so the
/// worst-case token cost is bounded at 3× a single turn.
pub const MAX_STREAM_ATTEMPTS: u8 = 3;

/// Base backoff in ms; actual delay is `BASE_BACKOFF_MS * 2^(attempt-1)` plus
/// up to 250 ms jitter, giving roughly 0.5/1/2/4/8 s between attempts.
const BASE_BACKOFF_MS: u64 = 500;

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
/// jitter: `BASE_BACKOFF_MS * 2^(attempt-1)` → 500/1000/2000/4000/8000 ms.
/// Extracted as a pure function so the growth curve is unit-testable.
pub fn backoff_millis(attempt: u8) -> u64 {
    BASE_BACKOFF_MS.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1) as u32))
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
    Duration::from_millis(exp + jitter)
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
    /// response was truncated. Previously treated as a silent success (bug);
    /// now retried, degrading to a best-effort `Completed` only when the budget
    /// is exhausted.
    Truncated,
    /// No decoded SSE event arrived within the idle window. Catches an upstream
    /// that keeps the connection alive with keep-alive heartbeats but delivers
    /// no content.
    IdleTimeout,
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

    /// The backoff curve doubles each attempt: 0.5/1/2/4/8 s for attempts 1–5.
    #[test]
    fn backoff_millis_doubles_each_attempt() {
        assert_eq!(backoff_millis(1), 500);
        assert_eq!(backoff_millis(2), 1000);
        assert_eq!(backoff_millis(3), 2000);
        assert_eq!(backoff_millis(4), 4000);
        assert_eq!(backoff_millis(5), 8000);
        assert_eq!(MAX_ATTEMPTS, 5);
        assert_eq!(MAX_STREAM_ATTEMPTS, 3);
    }

    #[test]
    fn backoff_duration_within_millis_plus_jitter() {
        // backoff_duration must be the pure backoff plus at most 250 ms of
        // jitter, so a caller can safely combine it with a Retry-After hint via
        // `Duration::max` without surprising bounds.
        for attempt in 1..=5u8 {
            let lo = backoff_millis(attempt);
            let hi = lo + 250;
            let d = backoff_duration(attempt);
            assert!(
                d >= Duration::from_millis(lo) && d <= Duration::from_millis(hi),
                "attempt {attempt}: backoff_duration {d:?} not within [{lo}, {hi}] ms"
            );
        }
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
            retry_decision(AttemptOutcome::Success, 1, 5),
            RetryDecision::Done
        );
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 5, 5),
            RetryDecision::Done
        );
    }

    /// A non-retryable error fails FAST on every attempt — never retries.
    #[test]
    fn retry_decision_non_retryable_fails_fast() {
        for attempt in 1..=5u8 {
            assert_eq!(
                retry_decision(AttemptOutcome::NonRetryableError, attempt, 5),
                RetryDecision::Fail,
                "non-retryable must fail fast at attempt {attempt}"
            );
        }
    }

    /// A retryable error retries while attempts remain and FAILS exactly when
    /// attempt == max (no sixth attempt). This is the off-by-one canary.
    #[test]
    fn retry_decision_retryable_retries_then_fails_at_max() {
        for attempt in 1..=4u8 {
            assert_eq!(
                retry_decision(AttemptOutcome::RetryableError, attempt, 5),
                RetryDecision::Retry,
                "attempt {attempt} (< max=5) should retry"
            );
        }
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 5, 5),
            RetryDecision::Fail,
            "attempt == max must fail (no attempt beyond max)"
        );
    }

    /// Replay a full retry-then-recover sequence through the policy: 2
    /// retryable failures then success.
    #[test]
    fn retry_decision_sequence_recover_on_third_attempt() {
        let max = 5u8;
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
