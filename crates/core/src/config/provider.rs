//! `provider` config block types (`Config::provider` / `Config::providers`).
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Pure serde structs; endpoint-resolution behavior stays in `config.rs`
//! (`Config::resolve_endpoint`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    /// Default model id for this provider (the part after the `/` prefix).
    #[serde(default)]
    pub model: Option<String>,
    /// Extra HTTP headers attached to every request to this provider. A header
    /// `value` may be a literal string or a `{VAR}` reference resolved from the
    /// environment at endpoint-resolution time (same convention as `api_key`).
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
}

/// A custom HTTP header applied to provider requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// Resolved provider endpoint: everything `ChatClient::new` needs to talk to
/// the model's provider. `headers` are env-resolved name/value pairs; a custom
/// header sharing a built-in name (e.g. `authorization`, `content-type`)
/// overrides the built-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub headers: Vec<(String, String)>,
}

/// Default `base_url` for the active provider. `pub(super)` because
/// `impl Default for Config` in the parent module pins the field explicitly
/// (keeping it in sync with this serde default).
pub(super) fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}
