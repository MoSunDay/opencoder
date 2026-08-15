use serde::{Deserialize, Serialize};

/// Durable workflow projection. Domain-specific state stays JSON so Store
/// remains independent from the orchestration crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWorkflowRecord {
    pub id: String,
    pub parent_session_id: String,
    pub status: String,
    pub spec_json: serde_json::Value,
    pub state_json: serde_json::Value,
    pub generation: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub terminal_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItemRecord {
    pub workflow_id: String,
    pub todo_id: String,
    pub ordinal: i64,
    pub status: String,
    pub attempt: i64,
    pub active_session_id: Option<String>,
    pub session_history: Vec<String>,
    pub result_json: Option<serde_json::Value>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoEventRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    pub workflow_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoWorkflowSummary {
    pub id: String,
    pub status: String,
    pub parent_session_id: String,
    pub generation: i64,
    pub updated_at: i64,
}
