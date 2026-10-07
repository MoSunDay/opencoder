//! Post-task repository memory maintenance in a store-less context copy.

use anyhow::{anyhow, Result};
use opencoder_core::{
    body_with_source, resolve_agent, skill, AgentKind, AgentMode, ApMode, Role, ToolFilter,
};

use super::{new_id, SessionEvent};
use crate::SessionState;

fn eligible_as(parent: &SessionState, kind: AgentKind) -> bool {
    parent.config.local_memory
        // External Codex owns its task lifecycle and model credentials. Starting
        // an Act child here silently switches harness and may have no provider.
        && parent.harness.harness != opencoder_core::harness::Harness::Codex
        && kind == AgentKind::Act
        && parent.agent.mode == AgentMode::Primary
        && parent.agent.name != "workflow"
        && !parent.id.starts_with("memory-")
}

/// Memory maintenance runs only for act-mode primary tasks: the
/// `local-memory` config switch must be on AND the session must be running
/// the act-kind agent — plan (read-only), command, workflow and subagent
/// sessions never update memory.
pub(super) fn eligible(parent: &SessionState) -> bool {
    eligible_as(parent, parent.agent.kind)
}

/// Eligibility for the run about to start with `user_text`. A leading
/// control command switches the agent *inside* the run (`/act task` from a
/// plan session executes in act mode), so the gate probes the switch
/// target's kind instead of the current agent. The post-run re-check in
/// [`completed`] remains the authority for the session's final state.
pub(super) fn eligible_for_run(parent: &SessionState, user_text: &str) -> bool {
    eligible_as(parent, run_kind(parent, user_text))
}

/// The agent kind the upcoming input will run under: the control-command
/// switch target when the input starts with one, else the current agent.
/// `/act_clear_context` from plan is the plan→act execution handoff and
/// converges the session to act; from other kinds it keeps the agent.
fn run_kind(parent: &SessionState, user_text: &str) -> AgentKind {
    match crate::control_cmd::split_control_prefix(user_text).map(|(cmd, _)| cmd) {
        Some(crate::control_cmd::ControlCmd::SwitchAgent(name)) => {
            resolve_agent(&name).map_or(parent.agent.kind, |agent| agent.kind)
        }
        Some(crate::control_cmd::ControlCmd::ClearContext)
            if parent.agent.kind == AgentKind::Plan =>
        {
            AgentKind::Act
        }
        _ => parent.agent.kind,
    }
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

/// The instruction handed to the memory-maintenance child. Doubles as the
/// display prompt of the TUI's foldable memory block (`SubagentStart`), so
/// what the user sees matches what the model received.
const MEMORY_INSTRUCTION: &str = "The main task is complete. Update repository local memory for this task using the active skill. Inspect the completed work and write only necessary memory changes. Do not redo the main task.";

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
    agent.tools = ToolFilter::Allow(vec![opencoder_core::platform::shell::tool_name().into()]);
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
    let id = child.id.clone();
    on_event(SessionEvent::Status("updating local memory".into()));
    // Echo the maintenance run as a foldable "memory" block instead of a bare
    // status line: the parent's transcript shows what memory maintenance
    // actually did (tool calls, output, text) by riding the exact routing
    // real subagents use — `SubagentStart`/`SubagentChild`/`SubagentEnd`
    // already reach the TUI block view, the web SSE relay and the headless
    // footer, so the display surfaces need no new plumbing.
    on_event(SessionEvent::SubagentStart {
        id: id.clone(),
        kind: "memory".into(),
        prompt: MEMORY_INSTRUCTION.into(),
        child_session_id: id.clone(),
    });
    let started = std::time::Instant::now();
    let mut child_error = None;
    let mut summary = String::new();
    let registry = super::registry::build_full_registry(&child).await;
    let run = super::entry::run_without_memory(
        &mut child,
        MEMORY_INSTRUCTION.into(),
        Vec::new(),
        &registry,
        |event| {
            if let SessionEvent::TextDelta(text) = &event {
                if summary.len() < 240 {
                    summary.push_str(text);
                }
            }
            if let SessionEvent::Error(error) = &event {
                child_error = Some(error.clone());
            }
            on_event(SessionEvent::SubagentChild {
                id: id.clone(),
                ev: Box::new(event),
            });
        },
    )
    .await;
    // Close the block on every exit path (child error, run failure, success):
    // an open block would wedge the parent's running-subagent counter until
    // the next `Done`.
    let failure = child_error
        .clone()
        .or_else(|| run.as_ref().err().map(|e| format!("{e:#}")));
    on_event(SessionEvent::SubagentEnd {
        id: id.clone(),
        ok: failure.is_none(),
        cancelled: false,
        // Success carries the elapsed time as a prefix (the sibling subagent
        // summary convention "({n} tool calls) {summary}"); failure keeps the
        // raw error text so the reason stays legible.
        summary: failure.unwrap_or_else(|| {
            format!(
                "({}) {}",
                super::execute::fmt_dur(started.elapsed()),
                summary.trim()
            )
        }),
    });
    if let Some(error) = child_error {
        return Err(anyhow!("local-memory update failed: {error}"));
    }
    run?;
    on_event(SessionEvent::Status("local memory updated".into()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};

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
            let config = Config {
                local_memory: true,
                ..Config::default()
            };
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
            let requests = client.requests();
            let memory_tools: Vec<_> = requests[1]
                .tools
                .iter()
                .filter_map(|tool| tool["function"]["name"].as_str())
                .collect();
            assert_eq!(memory_tools, [opencoder_core::platform::shell::tool_name()]);
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

    #[tokio::test]
    async fn memory_output_never_enters_the_parent_transcript() {
        let root = tempfile::tempdir().unwrap();
        skill::seed_builtin_skills_in(root.path()).unwrap();
        skill::with_execution(Some(root.path().to_path_buf()), async {
            let client = Arc::new(
                MockChatClient::new()
                    .push_script(vec![LlmEvent::Completed {
                        text: "done".into(),
                        tool_calls: Vec::new(),
                        usage: None,
                    }])
                    .push_script(vec![
                        LlmEvent::TextDelta("memory notes written".into()),
                        LlmEvent::Completed {
                            text: "memory notes written".into(),
                            tool_calls: Vec::new(),
                            usage: None,
                        },
                    ]),
            );
            let config = Config {
                local_memory: true,
                ..Config::default()
            };
            let mut parent = SessionState::new(
                "parent-memory-isolation-test",
                resolve_agent("act").unwrap(),
                config,
                client.clone() as Arc<dyn ChatStream>,
                root.path().to_path_buf(),
            );
            super::super::run(&mut parent, "complete task".into(), |_| {})
                .await
                .unwrap();
            assert_eq!(
                client.call_count(),
                2,
                "main task followed by the maintenance round"
            );
            assert_eq!(
                parent.messages.len(),
                2,
                "only the task's own User/Assistant pair remains"
            );
            assert_eq!(parent.messages[0].role, Role::User);
            assert_eq!(parent.messages[1].role, Role::Assistant);
            assert!(
                parent
                    .messages
                    .iter()
                    .all(|m| !m.text().contains("memory notes written")),
                "the maintenance delta never lands in any parent message"
            );
            assert!(
                parent
                    .messages
                    .iter()
                    .all(|m| !m.text().contains("Update repository local memory")),
                "the maintenance instruction never lands in any parent message"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn memory_run_echoes_its_progress_as_a_subagent_block() {
        let root = tempfile::tempdir().unwrap();
        skill::seed_builtin_skills_in(root.path()).unwrap();
        skill::with_execution(Some(root.path().to_path_buf()), async {
            let client = Arc::new(
                MockChatClient::new()
                    .push_script(vec![LlmEvent::Completed {
                        text: "done".into(),
                        tool_calls: Vec::new(),
                        usage: None,
                    }])
                    .push_script(vec![
                        LlmEvent::TextDelta("memory notes written".into()),
                        LlmEvent::Completed {
                            text: "memory updated".into(),
                            tool_calls: Vec::new(),
                            usage: Some(Usage {
                                input_tokens: 600,
                                output_tokens: 634,
                                total_tokens: 1234,
                                ..Default::default()
                            }),
                        },
                    ]),
            );
            let config = Config {
                local_memory: true,
                ..Config::default()
            };
            let mut parent = SessionState::new(
                "echo-memory-test",
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
            let block_id = events
                .iter()
                .find_map(|e| match e {
                    SessionEvent::SubagentStart {
                        id,
                        kind,
                        child_session_id,
                        ..
                    } if kind == "memory" => {
                        assert_eq!(id, child_session_id);
                        Some(id.clone())
                    }
                    _ => None,
                })
                .expect("memory maintenance opens a subagent block");
            assert!(block_id.starts_with("memory-"));
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    SessionEvent::SubagentChild { id, ev }
                        if id == &block_id && matches!(ev.as_ref(), SessionEvent::TextDelta(_))
                )),
                "the child's frames stream into the block"
            );
            assert!(
                events.iter().any(|e| matches!(
                    e,
                    SessionEvent::SubagentChild { id, ev }
                        if id == &block_id
                            && matches!(
                                ev.as_ref(),
                                SessionEvent::LlmUsage { total_tokens: 1234, .. }
                            )
                )),
                "the child's LlmUsage is forwarded wrapped as SubagentChild — \
                 the exact input the TUI folds into the parent tok cost"
            );
            let (ok, summary) = events
                .iter()
                .find_map(|e| match e {
                    SessionEvent::SubagentEnd {
                        id, ok, summary, ..
                    } if id == &block_id => Some((*ok, summary.clone())),
                    _ => None,
                })
                .expect("memory maintenance closes its block");
            assert!(ok, "the maintenance run succeeded");
            assert!(
                summary.starts_with('('),
                "the block footer leads with the timed duration, got: {summary}"
            );
            assert!(
                summary.contains("memory notes written"),
                "the block footer summarizes what the run said after the duration prefix"
            );
        })
        .await;
    }

    #[test]
    fn eligible_for_run_keys_on_the_control_switch_target() {
        let root = tempfile::tempdir().unwrap();
        let client = Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>;
        let config = Config {
            local_memory: true,
            ..Config::default()
        };
        let plan = SessionState::new(
            "probe-plan",
            resolve_agent("plan").unwrap(),
            config.clone(),
            client.clone(),
            root.path().to_path_buf(),
        );
        assert!(!super::eligible(&plan), "a plan session is not eligible");
        assert!(super::eligible_for_run(&plan, "/act complete task"));
        assert!(super::eligible_for_run(&plan, "/act_clear_context run it"));
        assert!(!super::eligible_for_run(&plan, "plain plan question"));

        let act = SessionState::new(
            "probe-act",
            resolve_agent("act").unwrap(),
            config,
            client,
            root.path().to_path_buf(),
        );
        assert!(super::eligible_for_run(&act, "do work"));
        assert!(!super::eligible_for_run(&act, "/plan review this"));
        assert!(super::eligible_for_run(&act, "/act_clear_context redo"));
    }

    #[tokio::test]
    async fn plan_mode_task_does_not_update_memory() {
        let root = tempfile::tempdir().unwrap();
        skill::seed_builtin_skills_in(root.path()).unwrap();
        skill::with_execution(Some(root.path().to_path_buf()), async {
            let client = Arc::new(
                MockChatClient::new().with_default(vec![LlmEvent::Completed {
                    text: "plan".into(),
                    tool_calls: Vec::new(),
                    usage: None,
                }]),
            );
            let config = Config {
                local_memory: true,
                ..Config::default()
            };
            let mut parent = SessionState::new(
                "plan-memory-test",
                resolve_agent("plan").unwrap(),
                config,
                client.clone() as Arc<dyn ChatStream>,
                root.path().to_path_buf(),
            );
            let mut events = Vec::new();
            super::super::run(&mut parent, "review the repo".into(), |event| {
                events.push(event)
            })
            .await
            .unwrap();
            assert_eq!(client.call_count(), 1, "plan task only, no memory run");
            assert_eq!(parent.messages.len(), 2, "no memory transcript appended");
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, SessionEvent::Done))
                    .count(),
                1,
                "exactly one Done without memory maintenance"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn compound_act_switch_from_plan_still_updates_memory() {
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
            let config = Config {
                local_memory: true,
                ..Config::default()
            };
            let mut parent = SessionState::new(
                "plan-switch-memory-test",
                resolve_agent("plan").unwrap(),
                config,
                client.clone() as Arc<dyn ChatStream>,
                root.path().to_path_buf(),
            );
            let mut events = Vec::new();
            super::super::run(&mut parent, "/act complete task".into(), |event| {
                events.push(event)
            })
            .await
            .unwrap();
            assert_eq!(
                parent.agent.kind,
                opencoder_core::AgentKind::Act,
                "the compound input switched the session to act"
            );
            assert_eq!(client.call_count(), 2, "act task followed by memory run");
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, SessionEvent::Done))
                    .count(),
                1,
                "the parent ends once, after memory maintenance"
            );
        })
        .await;
    }
}
