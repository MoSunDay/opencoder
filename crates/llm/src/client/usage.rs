use crate::Usage;
use serde_json::Value;

pub(crate) fn parse_usage(u: &Value) -> Usage {
    fn get_tokens(u: &Value, key: &str) -> Option<u64> {
        u.get(key)
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
    }
    let input_tokens = get_tokens(u, "input_tokens")
        .or_else(|| get_tokens(u, "prompt_tokens"))
        .unwrap_or_default();
    let output_tokens = get_tokens(u, "output_tokens")
        .or_else(|| get_tokens(u, "completion_tokens"))
        .unwrap_or_default();
    let total_tokens = get_tokens(u, "total_tokens")
        .filter(|&t| t != 0)
        .unwrap_or(input_tokens.saturating_add(output_tokens));

    // Prompt-caching accounting: accept every provider naming variant
    // (cache_read_input_tokens | cache_read | prompt_tokens_details.cached_tokens;
    //  cache_creation_input_tokens | cache_write) and normalize to two fields.
    let cache_read_tokens = first_u64(u, &["cache_read_input_tokens", "cache_read"])
        .or_else(|| {
            u.get("input_tokens_details")
                .or_else(|| u.get("prompt_tokens_details"))
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        })
        .unwrap_or_default();
    let cache_creation_tokens = first_u64(u, &["cache_creation_input_tokens", "cache_write"])
        .or_else(|| {
            u.pointer("/input_tokens_details/cache_write_tokens")
                .and_then(Value::as_u64)
        })
        .unwrap_or_default();

    Usage {
        reasoning_tokens: u
            .pointer("/output_tokens_details/reasoning_tokens")
            .or_else(|| u.pointer("/completion_tokens_details/reasoning_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        input_tokens,
        output_tokens,
        total_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    }
}

/// Return the first `u64` found under any of `keys` in `obj`, or `None`.
/// Used by `parse_usage` to collapse provider-specific cache-field aliases
/// (checked in priority order) into one normalized value.
pub(super) fn first_u64(obj: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| {
        obj.get(*k)
            .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
    })
}
