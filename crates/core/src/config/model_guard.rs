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

/// Validate every persisted model selector before a save or Env activation.
/// Nested mode selectors use the same predicate as the legacy top-level model.
pub(super) fn validate_model_values(root: &serde_json::Value) -> crate::Result<()> {
    validate_one("model", root.get("model").and_then(|v| v.as_str()))?;
    if let Some(modes) = root.get("modes").and_then(|v| v.as_object()) {
        for mode in ["shared", "act", "plan"] {
            let value = modes
                .get(mode)
                .and_then(|v| v.as_object())
                .and_then(|v| v.get("model"));
            if let Some(value) = value {
                if !value.is_null() && !value.is_string() {
                    return Err(crate::CoreError::Config(format!(
                        "refusing to write malformed `modes.{mode}.model`: expected a string or null"
                    )));
                }
            }
            validate_one(
                &format!("modes.{mode}.model"),
                value.and_then(|v| v.as_str()),
            )?;
        }
    }
    Ok(())
}

fn validate_one(field: &str, model: Option<&str>) -> crate::Result<()> {
    if let Some(model) = model.filter(|model| is_suspicious_model(model)) {
        return Err(crate::CoreError::Config(format!(
            "refusing to write malformed `{field}` value `{model}`: expected \
             `provider/model` with each side at least 2 chars (e.g. \
             `openai/gpt-4o`); edit the `{field}` field in your config file"
        )));
    }
    Ok(())
}
