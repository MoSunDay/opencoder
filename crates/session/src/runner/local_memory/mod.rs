//! Post-task repository memory maintenance in a store-less context copy.

use anyhow::{anyhow, Result};
use opencoder_core::{body_with_source, resolve_agent, skill, AgentMode, ApMode, Role, ToolFilter};

use super::{new_id, SessionEvent};
use crate::SessionState;

pub(super) fn eligible(parent: &SessionState) -> bool {
    parent.config.local_memory
        && parent.agent.mode == AgentMode::Primary
        && parent.agent.name != "workflow"
        && !parent.id.starts_with("memory-")
}

fn completed(parent: &SessionState, baseline: usize) -> bool {
    eligible(parent)
        && parent
            .cancel
            .as_ref()
            .is_none_or(|cancel| !cancel.is_cancelled())
        && parent.messages[baseline.min(parent.messages.len())..]
            .iter()
            .any(|message| message.role == Role::Assistant)
}

pub(super) async fn after_task(
    parent: &SessionState,
    baseline: usize,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    if !completed(parent, baseline) {
        return Ok(());
    }
    let pack = skill::discover()
        .into_iter()
        .find(|pack| pack.name == "repo-local-memory")
        .ok_or_else(|| anyhow!("local-memory is enabled but repo-local-memory skill is missing"))?;
    let mut agent = resolve_agent("act").ok_or_else(|| anyhow!("act agent is unavailable"))?;
    agent.tools = ToolFilter::Allow(vec!["bash".into()]);
    let mut config = parent.config.clone();
    config.local_memory = false;
    config.autopilot.mode = ApMode::Off;
    config.compaction.auto = false;
    let mut child = SessionState::new(
        format!("memory-{}", new_id()),
        agent,
        config,
        parent.client.clone(),
        parent.working_dir.clone(),
    );
    child.model = parent.model.clone();
    child.messages = parent.messages.clone();
    child.env_passthrough = parent.env_passthrough.clone();
    child.set_skill(Some(body_with_source(&pack)));
    child.set_active_skill_names([pack.name].into_iter().collect());
    on_event(SessionEvent::Status("updating local memory".into()));
    let mut child_error = None;
    let registry = super::registry::build_full_registry(&child).await;
    super::entry::run_without_memory(
        &mut child,
        "The main task is complete. Update repository local memory for this task using the active skill. Inspect the completed work and write only necessary memory changes. Do not redo the main task.".into(),
        Vec::new(),
        &registry,
        |event| match event {
            SessionEvent::LlmUsage { .. } => on_event(event),
            SessionEvent::Error(error) => child_error = Some(error),
            _ => {}
        },
    ).await?;
    if let Some(error) = child_error {
        return Err(anyhow!("local-memory update failed: {error}"));
    }
    on_event(SessionEvent::Status("local memory updated".into()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};

    use super::*;

    #[tokio::test]
    async fn enabled_memory_uses_a_context_copy_after_main_completion() {
        let root = tempfile::tempdir().unwrap();
        skill::seed_builtin_skills_in(root.path()).unwrap();
        skill::with_execution(Some(root.path().to_path_buf()), async {
            let client = Arc::new(
                MockChatClient::new().with_default(vec![LlmEvent::Completed {
                    text: "done".into(),
                    tool_calls: Vec::new(),
                    usage: None,
                }]),
            );
            let mut config = Config::default();
            config.local_memory = true;
            let mut parent = SessionState::new(
                "main-memory-test",
                resolve_agent("act").unwrap(),
                config,
                client.clone() as Arc<dyn ChatStream>,
                root.path().to_path_buf(),
            );
            let mut events = Vec::new();
            super::super::run(&mut parent, "complete task".into(), |event| {
                events.push(event)
            })
            .await
            .unwrap();
            assert_eq!(client.call_count(), 2, "main task followed by memory run");
            assert_eq!(parent.messages.len(), 2, "memory transcript stays separate");
            assert_eq!(parent.messages[0].role, Role::User);
            assert!(matches!(events.last(), Some(SessionEvent::Done)));
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, SessionEvent::Done))
                    .count(),
                1,
                "the parent ends after memory maintenance"
            );
        })
        .await;
    }
}
