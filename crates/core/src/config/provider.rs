//! `provider` config block types (`Config::provider` / `Config::providers`).
//!
//! Extracted from `config.rs` so the main module stays under the line gate.
//! Pure serde structs; endpoint-resolution behavior stays in `config.rs`
//! (`Config::resolve_endpoint`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    /// Default model id for this provider (the part after the `/` prefix).
    #[serde(default)]
    pub model: Option<String>,
    /// Sampling temperature pinned on every request to this provider. Some
    /// backends only accept one fixed temperature (e.g. reasoning models
    /// locked to `1`); when set it overrides any per-purpose default
    /// (compaction/title/verify) and the conversation default (unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Extra HTTP headers attached to every request to this provider. A header
    /// `value` may be a literal string or a `{VAR}` reference resolved from the
    /// environment at endpoint-resolution time (same convention as `api_key`).
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
}

/// A custom HTTP header applied to provider requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub protocol: crate::ProviderProtocol,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub headers: Vec<(String, String)>,
}

impl crate::Config {
    /// Resolve the pinned sampling temperature for a provider name:
    /// `providers[name].temperature` -> legacy `provider.temperature` ->
    /// `None` (per-purpose defaults / unset apply). Pinned by backends that
    /// only accept a single temperature value.
    pub fn temperature_for(&self, name: &str) -> Option<f64> {
        self.provider_for(name)
            .and_then(|p| p.temperature)
            .or(self.provider.temperature)
    }
}

pub(super) fn default_protocol() -> String {
    "chat_completions".into()
}

/// Default `base_url` for the active provider. `pub(super)` because
/// `impl Default for Config` in the parent module pins the field explicitly
/// (keeping it in sync with this serde default).
pub(super) fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            protocol: default_protocol(),
            base_url: String::new(),
            api_key: None,
            model: None,
            temperature: None,
            headers: Vec::new(),
        }
    }
}
