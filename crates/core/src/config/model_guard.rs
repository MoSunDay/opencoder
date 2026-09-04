//! Guards against malformed `model` values.
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Pure predicates only: no config I/O, no mutation — `Config::load` and
//! `Config::save` (plus the cli/web model menus) consume them.

/// Pure predicate: is the `model` string malformed (empty, too short on
/// either side of the `/`, or too short unscoped)? `pub` for cli/web checks.
///
/// Warn target (without rewriting) when the configured `model` looks like a
/// stale or malformed value that would silently break requests: legacy values
/// such as single-char or placeholder strings must not be silently written
/// back to config.json.
pub fn is_suspicious_model(model: &str) -> bool {
    if model.is_empty() {
        return true;
    }
    match model.split_once('/') {
        Some((provider, mid)) => provider.len() < 2 || mid.len() < 2,
        None => model.len() < 3,
    }
}

/// Log (only) when `model` looks malformed; never mutates the user's config.
pub(super) fn warn_if_suspicious_model(model: &str) {
    if is_suspicious_model(model) {
        tracing::warn!(
            model = %model,
            "config `model` looks malformed (expected `provider/model`, e.g. `openai/gpt-4o`); fix the `model` field in your config file or set the matching env var if this is a stale value"
        );
    }
}
