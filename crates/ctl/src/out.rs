//! Output conventions: stdout carries exactly one machine-readable JSON
//! document per invocation (agents parse it); human-facing notes go to
//! stderr and never mix into stdout.

use serde_json::Value;

/// Print a JSON document to stdout (pretty, still a single parseable doc).
pub fn json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

/// Print one compact JSON line to stdout and flush (SSE frames).
pub fn json_line(value: &Value) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", serde_json::to_string(value).unwrap_or_default());
    let _ = out.flush();
}

/// Human-facing note on stderr; ignored by agents.
pub fn note(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}

/// Structured failure on stderr: `{"status":..,"error":..}`.
pub fn fail(status: u16, error: &str) {
    eprintln!("{}", serde_json::json!({"status": status, "error": error}));
}

/// Structured transport failure (no HTTP status available).
pub fn fail_transport(error: &str) {
    eprintln!("{}", serde_json::json!({"status": 0, "error": error}));
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_line_is_single_compact_line() {
        // Regression guard for the agent contract: one line, no pretty breaks.
        let value = serde_json::json!({"a": 1, "b": [1, 2]});
        let text = serde_json::to_string(&value).unwrap();
        assert!(!text.contains('\n'));
        assert_eq!(text, r#"{"a":1,"b":[1,2]}"#);
    }
}
