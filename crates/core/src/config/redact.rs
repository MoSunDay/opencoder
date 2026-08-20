//! Redact secret-bearing fields from arbitrary config-shaped JSON.
//!
//! Used wherever a config (or a config patch / env capture preview) is echoed
//! back to a human or an untrusted surface: the `api_key` field is the one
//! credential `ProviderConfig` can carry, and it appears both at the
//! top-level `provider` object and inside every `providers.<name>` registry
//! entry. Redaction is structural — it walks the whole tree, so any nesting
//! (arrays of providers, future provider groups) is covered without the
//! caller having to know the schema.

use serde_json::Value;

/// Object key whose string value is treated as a credential.
const SECRET_KEY: &str = "api_key";

/// Number of leading characters kept when masking a long secret — enough for
/// the user to recognise "their" key, never enough to replay it.
const KEEP_PREFIX: usize = 4;

/// Mask a single secret string: first 4 chars + `"***"` when longer than 4
/// chars, else plain `"***"`. Pure.
fn mask(secret: &str) -> String {
    if secret.chars().count() > KEEP_PREFIX {
        let prefix: String = secret.chars().take(KEEP_PREFIX).collect();
        format!("{prefix}***")
    } else {
        "***".to_string()
    }
}

/// Deep-copy `value` with every object key exactly `"api_key"` whose value is
/// a string replaced by its masked form (`sk-abcd123…` → `sk-a***`; anything
/// ≤4 chars → `"***"`). Non-string `api_key` values (e.g. `null`) and every
/// other key are copied through unchanged. Pure: input is never mutated.
pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let redacted = if k == SECRET_KEY {
                        match v {
                            Value::String(s) => Value::String(mask(s)),
                            // Non-string api_key (null/number/bool/object):
                            // nothing recognisable to leak — copy as-is.
                            other => other.clone(),
                        }
                    } else {
                        redact_json(v)
                    };
                    (k.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_json;
    use serde_json::{json, Value};

    #[test]
    fn long_api_key_is_masked_with_first_four_chars() {
        let v = json!({ "api_key": "sk-abcd-1234-xyz" });
        assert_eq!(redact_json(&v), json!({ "api_key": "sk-a***" }));
    }

    #[test]
    fn short_api_key_is_fully_masked() {
        // Exactly 4 chars and shorter: not "longer than 4" → plain "***".
        assert_eq!(
            redact_json(&json!({ "api_key": "abcd" })),
            json!({ "api_key": "***" })
        );
        assert_eq!(
            redact_json(&json!({ "api_key": "k" })),
            json!({ "api_key": "***" })
        );
        assert_eq!(
            redact_json(&json!({ "api_key": "" })),
            json!({ "api_key": "***" })
        );
        // 5 chars is the first masked-with-prefix case.
        assert_eq!(
            redact_json(&json!({ "api_key": "abcde" })),
            json!({ "api_key": "abcd***" })
        );
    }

    #[test]
    fn deep_nesting_object_in_array_in_object_is_redacted() {
        let v = json!({
            "providers": {
                "deepseek": { "base_url": "https://x/v1", "api_key": "dk-secret-secret" },
                "openai": { "api_key": "oai-secret-secret" }
            },
            "list": [
                { "api_key": "arr-secret-secret" },
                { "name": "no-key-here" }
            ]
        });
        let out = redact_json(&v);
        assert_eq!(out["providers"]["deepseek"]["api_key"], "dk-s***");
        assert_eq!(out["providers"]["openai"]["api_key"], "oai-***");
        assert_eq!(out["list"][0]["api_key"], "arr-***");
    }

    #[test]
    fn siblings_are_untouched_and_input_not_mutated() {
        let v = json!({
            "model": "openai/gpt-4o",
            "api_key": "sk-secret-secret",
            "provider": { "base_url": "https://api.openai.com/v1", "api_key": "sk-other-secret" },
            "n": 7,
            "flag": true
        });
        let original = v.clone();
        let out = redact_json(&v);
        assert_eq!(v, original, "redact_json must not mutate its input");
        assert_eq!(out["model"], "openai/gpt-4o");
        assert_eq!(out["provider"]["base_url"], "https://api.openai.com/v1");
        assert_eq!(out["n"], 7);
        assert_eq!(out["flag"], true);
        assert_eq!(out["api_key"], "sk-s***");
        assert_eq!(out["provider"]["api_key"], "sk-o***");
    }

    #[test]
    fn non_string_api_key_values_pass_through() {
        let v =
            json!({ "api_key": null, "nested": { "api_key": 1234 }, "api_keyb": "not-the-key" });
        assert_eq!(
            redact_json(&v),
            v,
            "null/number api_key and lookalike keys stay as-is"
        );
    }

    #[test]
    fn non_object_roots_pass_through() {
        assert_eq!(redact_json(&json!("api_key")), json!("api_key"));
        assert_eq!(redact_json(&json!([1, "two"])), json!([1, "two"]));
        assert_eq!(redact_json(&Value::Null), Value::Null);
    }
}
