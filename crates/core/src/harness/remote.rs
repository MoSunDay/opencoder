use crate::fleet::ExecutionKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConnection {
    pub enabled: bool,
    pub url: String,
}

/// Public projection of a registered Agent/Operator capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCapability {
    pub id: String,
    pub kind: ExecutionKind,
    pub target: String,
    pub summary: String,
}

/// Local bookmark of a server-owned execution. The execution ID is the local
/// session ID too. `created` changes only after Server accepts the first input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSession {
    pub server_url: String,
    pub capability: ServerCapability,
    #[serde(default)]
    pub created: bool,
    /// Durable first request allows retry after a lost admission response.
    #[serde(default)]
    pub initial_input: Option<serde_json::Value>,
}

impl RemoteSession {
    pub fn label(&self) -> String {
        format!("{}:{}", self.capability.kind.prefix(), self.capability.id)
    }
}
