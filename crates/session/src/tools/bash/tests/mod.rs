use super::*;
use opencoder_core::ToolContext;
use serde_json::json;
use std::ops::{Deref, DerefMut};

struct TestContext {
    context: ToolContext,
    _profile_dir: tempfile::TempDir,
}

impl Deref for TestContext {
    type Target = ToolContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl DerefMut for TestContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

fn ctx() -> TestContext {
    // Keep login-profile commands and output outside the command lifecycle
    // under test. The fixture owns the directory until the tool call finishes.
    let profile_dir = tempfile::tempdir().unwrap();
    let context = ToolContext {
        extra_env: vec![(
            "HOME".into(),
            profile_dir.path().to_string_lossy().into_owned(),
        )],
        session_id: "test".into(),
        message_id: "test".into(),
        agent: "act".into(),
        working_dir: std::env::current_dir().unwrap(),
        max_output: 100_000,
        proxy: None,
        tools_path: None,
    };
    TestContext {
        context,
        _profile_dir: profile_dir,
    }
}

mod completion;
mod lifecycle;
