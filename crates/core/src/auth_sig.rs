//! HMAC-SHA256 request-signing protocol shared by the daemon server, the SPA
//! and execution nodes.
//!
//! Pure functions only: canonical-string construction, signing, and offline
//! verification (timestamp window + constant-time compare). The stateful
//! replay dedup lives at the middleware layer (`opencoder-web`), which is the
//! only place with process memory.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Requests whose timestamp deviates from the receiver's clock by more than
/// this are rejected outright. Bounds both infinite replay and long-range
/// forgery; `/api/time` lets browser clients compensate for clock skew.
pub const REPLAY_WINDOW_MS: i64 = 300_000;

/// Hex HMAC over the canonical string.
pub const SIG_HEADER: &str = "x-sig";
/// Unix milliseconds (stringified) of the sender's clock.
pub const TS_HEADER: &str = "x-sig-timestamp";

/// Canonical signing string:
/// `"{METHOD}\n{path_and_query}\n{ts}\n{sha256_hex(body)}"`.
///
/// The method/path/query/body hash pin every mutable aspect of the request;
/// only the headers (carrying ts + sig) stay outside, which is safe because
/// they are the signature itself. Newlines are impossible in method names and
/// cannot start a path, making the concatenation unambiguous.
pub fn canonical(method: &str, path_and_query: &str, ts_ms: i64, body: &[u8]) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path_and_query,
        ts_ms,
        sha256_hex(body)
    )
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

/// HMAC-SHA256 over the canonical string, keyed by the shared token; lowercase hex.
pub fn sign_hex(secret: &str, canonical: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(canonical.as_bytes());
    hex_encode(&mac.finalize().into_bytes())
}

/// Verification failure modes. Middleware maps both to 401.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigError {
    /// `|now - ts| > REPLAY_WINDOW_MS`.
    TimestampOutOfRange,
    /// Signature mismatch (covers missing/garbled inputs after parsing).
    Mismatch,
}

/// Full offline verification: window first (cheap), then constant-time
/// signature comparison. NOT replay detection — the same valid signature must
/// be deduped by the caller's cache.
pub fn verify(
    secret: &str,
    method: &str,
    path_and_query: &str,
    ts_ms: i64,
    now_ms: i64,
    body: &[u8],
    sig_hex: &str,
) -> Result<(), SigError> {
    // saturating_sub + saturating_abs keep the window check total: a hostile
    // `ts_ms = i64::MIN` made `now_ms - ts_ms` overflow i64 (debug builds
    // panicked — a one-request DoS — while release builds wrapped and could
    // skip the window entirely). Both extremes saturate instead.
    let delta = now_ms.saturating_sub(ts_ms).saturating_abs();
    if delta > REPLAY_WINDOW_MS {
        return Err(SigError::TimestampOutOfRange);
    }
    let canon = canonical(method, path_and_query, ts_ms, body);
    let expected = sign_hex(secret, &canon);
    if ct_eq(expected.as_bytes(), sig_hex.trim().as_bytes()) {
        Ok(())
    } else {
        Err(SigError::Mismatch)
    }
}

/// Length-safe constant-time byte equality (leaks only the length).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Lowercase hex encoding (hand-rolled to keep core dependency-light).
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256("") — anchors the canonical string against accidental format drift.
    const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn canonical_pins_method_path_ts_and_body_hash() {
        let c = canonical("get", "/api/nodes?x=1", 42, b"");
        assert_eq!(c, format!("GET\n/api/nodes?x=1\n42\n{EMPTY_SHA}"));
        // Method is normalized so a lowercase sender cannot split the space.
        assert_eq!(
            canonical("GET", "/p", 1, b""),
            canonical("get", "/p", 1, b"")
        );
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let body = br#"{"prompt":"hi"}"#;
        let canon = canonical("POST", "/api/nodes/n1/tasks", 1_000, body);
        let sig = sign_hex("secret", &canon);
        assert_eq!(sig.len(), 64);
        assert_eq!(
            verify(
                "secret",
                "POST",
                "/api/nodes/n1/tasks",
                1_000,
                1_000,
                body,
                &sig
            ),
            Ok(())
        );
    }

    #[test]
    fn window_boundaries_are_inclusive() {
        let sig = |ts: i64| sign_hex("s", &canonical("GET", "/p", ts, b""));
        assert_eq!(
            verify("s", "GET", "/p", 0, REPLAY_WINDOW_MS, b"", &sig(0)),
            Ok(())
        );
        assert_eq!(
            verify("s", "GET", "/p", 0, REPLAY_WINDOW_MS + 1, b"", &sig(0)),
            Err(SigError::TimestampOutOfRange)
        );
        // Future-dated stamps get the same symmetric treatment.
        assert_eq!(
            verify(
                "s",
                "GET",
                "/p",
                1_000,
                1_000 - REPLAY_WINDOW_MS,
                b"",
                &sig(1_000)
            ),
            Ok(())
        );
        assert_eq!(
            verify(
                "s",
                "GET",
                "/p",
                1_000,
                1_000 - REPLAY_WINDOW_MS - 1,
                b"",
                &sig(1_000)
            ),
            Err(SigError::TimestampOutOfRange)
        );
    }

    #[test]
    fn extreme_timestamps_reject_without_overflow() {
        let sig_zero = sign_hex("s", &canonical("GET", "/p", 0, b""));
        let sig_min = sign_hex("s", &canonical("GET", "/p", i64::MIN, b""));
        let sig_max = sign_hex("s", &canonical("GET", "/p", i64::MAX, b""));
        // `0 - i64::MIN` overflows i64: the subtraction must saturate, not
        // panic (debug) or wrap past the window (release).
        assert_eq!(
            verify("s", "GET", "/p", i64::MIN, 0, b"", &sig_min),
            Err(SigError::TimestampOutOfRange)
        );
        assert_eq!(
            verify("s", "GET", "/p", i64::MAX, 0, b"", &sig_max),
            Err(SigError::TimestampOutOfRange)
        );
        // Extreme receiver clocks are the same computation mirrored.
        assert_eq!(
            verify("s", "GET", "/p", 0, i64::MAX, b"", &sig_zero),
            Err(SigError::TimestampOutOfRange)
        );
        assert_eq!(
            verify("s", "GET", "/p", 0, i64::MIN, b"", &sig_zero),
            Err(SigError::TimestampOutOfRange)
        );
        // Control: a normal in-window stamp still verifies.
        let sig_window = sign_hex("s", &canonical("GET", "/p", 1_000, b""));
        assert_eq!(
            verify("s", "GET", "/p", 1_000, 1_000, b"", &sig_window),
            Ok(())
        );
    }

    #[test]
    fn wrong_secret_is_mismatch() {
        let sig = sign_hex("right", &canonical("GET", "/p", 1, b""));
        assert_eq!(
            verify("wrong", "GET", "/p", 1, 1, b"", &sig),
            Err(SigError::Mismatch)
        );
    }

    #[test]
    fn tampered_body_is_mismatch() {
        let sig = sign_hex("s", &canonical("POST", "/p", 1, b"original"));
        assert_eq!(
            verify("s", "POST", "/p", 1, 1, b"tampered", &sig),
            Err(SigError::Mismatch)
        );
    }

    #[test]
    fn tampered_path_or_method_is_mismatch() {
        let sig = sign_hex("s", &canonical("GET", "/api/a", 1, b""));
        assert_eq!(
            verify("s", "GET", "/api/b", 1, 1, b"", &sig),
            Err(SigError::Mismatch)
        );
        assert_eq!(
            verify("s", "POST", "/api/a", 1, 1, b"", &sig),
            Err(SigError::Mismatch)
        );
        // A differing query string is part of the signed material too.
        assert_eq!(
            verify("s", "GET", "/api/a?x=1", 1, 1, b"", &sig),
            Err(SigError::Mismatch)
        );
    }

    #[test]
    fn query_string_is_signed() {
        let sig = sign_hex(
            "s",
            &canonical("GET", "/api/nodes/tasks/claim?node_id=n1", 1, b""),
        );
        assert_eq!(
            verify(
                "s",
                "GET",
                "/api/nodes/tasks/claim?node_id=n2",
                1,
                1,
                b"",
                &sig
            ),
            Err(SigError::Mismatch)
        );
        assert_eq!(
            verify(
                "s",
                "GET",
                "/api/nodes/tasks/claim?node_id=n1",
                1,
                1,
                b"",
                &sig
            ),
            Ok(())
        );
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        assert_eq!(sha256_hex(b""), EMPTY_SHA);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
