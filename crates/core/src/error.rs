use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("config error: {0}")]
    Config(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("permission denied: {0}")]
    Permission(String),
}
