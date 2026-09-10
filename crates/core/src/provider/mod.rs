//! Provider-independent routing and opaque protocol state.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    #[default]
    ChatCompletions,
    Responses,
}

impl ProviderProtocol {
    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "chat_completions" => Ok(Self::ChatCompletions),
            "responses" => Ok(Self::Responses),
            _ => Err(crate::CoreError::Config(format!(
                "invalid provider protocol `{value}`: expected chat_completions or responses"
            ))),
        }
    }
}

/// Original output items are replayed only to the endpoint/model that issued
/// them. Kept separate from display blocks so formatting cannot corrupt them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderState {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub output: Vec<Value>,
}

/// Validate before merging/writing so invalid protocol values cannot disappear
/// in the permissive legacy configuration merger. Null means delete a key.
pub fn validate_protocol_patch(value: &Value) -> crate::Result<()> {
    let legacy = value.get("provider").into_iter();
    let named = value
        .get("providers")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|providers| providers.values());
    for provider in legacy.chain(named) {
        if let Some(protocol) = provider.get("protocol").filter(|v| !v.is_null()) {
            let name = protocol.as_str().ok_or_else(|| {
                crate::CoreError::Config("provider protocol must be a string".into())
            })?;
            ProviderProtocol::parse(name)?;
        }
    }
    Ok(())
}
