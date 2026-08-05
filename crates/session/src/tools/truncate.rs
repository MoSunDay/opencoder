//! UTF-8-safe body-truncation helper.
//!
//! Lives here (default-compiled, unlike the feature-gated `web_fetch` tool) so
//! the truncation logic and its regression test run in the standard
//! `cargo test --workspace` gate. `web_fetch` renders a URL with obscura and
//! then feeds the extracted text through [`truncate_body`].

use super::web_read::BODY_LIMIT;

/// Cap `text` at [`BODY_LIMIT`] bytes, trimming on a UTF-8 char boundary.
///
/// `String::truncate` panics unless the index is on a char boundary, and
/// `BODY_LIMIT` is a fixed byte count that -- on large non-ASCII pages (CJK,
/// emoji) -- almost always lands in the middle of a multibyte char. Back up to
/// the nearest boundary before truncating so the fetch never panics.
#[cfg_attr(not(feature = "browser"), allow(dead_code))]
pub(crate) fn truncate_body(text: &mut String) {
    if text.len() > BODY_LIMIT {
        let mut end = BODY_LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n\n[truncated at 2 MB]");
    }
}

#[cfg(test)]
mod tests {
    use super::{super::web_read::BODY_LIMIT, truncate_body};

    /// Regression: truncating a body past `BODY_LIMIT` must not panic when the
    /// `BODY_LIMIT` cut point falls mid-multibyte-char. A plain
    /// `String::truncate(BODY_LIMIT)` panics off a char boundary for large
    /// non-ASCII pages (CJK / emoji).
    #[test]
    fn truncate_body_respects_utf8_char_boundary() {
        // U+4E2D is a 3-byte char. Repeating it yields a string whose only char
        // boundaries are at multiples of 3. BODY_LIMIT (2 * 1024 * 1024) % 3
        // == 2, so byte BODY_LIMIT sits inside a char -> not a boundary.
        let unit = "\u{4e2d}".repeat(4096); // 12_288 bytes
        let mut text = unit.repeat(BODY_LIMIT / unit.len() + 2);
        assert!(text.len() > BODY_LIMIT, "precondition: must exceed cap");
        assert!(
            !text.is_char_boundary(BODY_LIMIT),
            "precondition: BODY_LIMIT must land mid-char (got a boundary)"
        );

        // Must not panic (plain truncate would) and stays valid UTF-8.
        truncate_body(&mut text);

        // The body is capped at BODY_LIMIT; the truncation marker is then
        // appended, so the *content* (everything before the marker) is what the
        // cap applies to.
        const MARKER: &str = "\n\n[truncated at 2 MB]";
        assert!(text.ends_with(MARKER), "truncation marker should be appended");
        let content_len = text.len() - MARKER.len();
        assert!(
            content_len <= BODY_LIMIT,
            "capped content must not exceed BODY_LIMIT: {content_len} > {BODY_LIMIT}"
        );
        // The marker must start on a char boundary (proves we backed up).
        assert!(text.is_char_boundary(content_len));
        assert!(std::str::from_utf8(text.as_bytes()).is_ok(), "result is valid UTF-8");
    }
}
