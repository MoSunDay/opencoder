use super::*;
use opencoder_core::ToolContext;
use serde_json::json;

fn ctx() -> ToolContext {
    ToolContext {
        extra_env: Vec::new(),
        session_id: "test".into(),
        message_id: "test".into(),
        agent: "act".into(),
        working_dir: std::env::current_dir().unwrap(),
        max_output: 100_000,
        proxy: None,
        tools_path: None,
    }
}

mod completion;
mod lifecycle;
