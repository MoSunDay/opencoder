use std::collections::HashMap;
use std::time::Duration;

use opencoder_core::{AgentKind, ToolArc, ToolContext, ToolOutput};
use opencoder_llm::tool_call::CompletedToolCall;

use crate::SessionState;

use super::event::{Sink, MAX_OUTPUT};
use super::steer::await_cancel;
use super::subagent::run_subagent;

/// Safety-net timeout for a single leaf-tool execution. Prevents a hung tool
/// (e.g. an ssh_pty tmux call that never returns, a stalled web_fetch, or a
/// browser/computer-use tool whose future never resolves) from freezing the
/// run loop forever. Generous enough that legitimate long-running tools are
/// unaffected — `bash` caps itself at 120 s — and the `task` subagent is
/// exempt entirely (it returns before this guard is reached, since a child
/// session may legitimately run for many minutes). Pairs with the per-read
/// LLM idle timeout (`DEFAULT_READ_TIMEOUT`); both are last-resort guards,
/// not expected to fire in normal operation.
pub(super) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(600);

pub(super) async fn execute_call(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
) -> ToolOutput {
    execute_call_with_timeout(tc, session, registry, sink, DEFAULT_TOOL_TIMEOUT).await
}

/// Like [`execute_call`] but with an injectable timeout, so the safety net is
/// unit-testable with a tiny timeout instead of waiting the full 10 minutes.
pub(super) async fn execute_call_with_timeout(
    tc: &CompletedToolCall,
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    sink: &Sink<'_>,
    timeout: Duration,
) -> ToolOutput {
    if tc.name == "task" {
        // The subagent runs as a child session and may legitimately take many
        // minutes, so it is exempt from the leaf-tool `DEFAULT_TOOL_TIMEOUT`.
        // It still gets its own (generous) deadline + the cancel guard so a
        // wedged child cannot freeze the run loop forever, and an interrupt is
        // honored promptly. Early-returns: the `task` tool never reaches the
        // generic registry-dispatch path below.
        let task_dur = session.config.task_timeout();
        let sub = run_subagent(tc.input.clone(), tc.id.clone(), session, registry, sink);
        let mut cancel_fut = std::pin::pin!(await_cancel(session));
        let mut deadline = std::pin::pin!(tokio::time::sleep(task_dur));
        return tokio::select! {
            biased;
            _ = &mut cancel_fut => ToolOutput::err("interrupted"),
            _ = &mut deadline => ToolOutput::err(format!(
                "subagent timed out after {} without completing",
                fmt_dur(task_dur)
            )),
            o = sub => o,
        };
    }
    // Plan-mode bash write guard: classify the command and block mutating
    // operations, returning a descriptive error to the model so it can adapt.
    if tc.name == "bash" && session.agent.kind == AgentKind::Plan {
        let cmd = tc
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let crate::bash_guard::BashVerdict::WriteBlocked(reason) =
            crate::bash_guard::classify(cmd)
        {
            return ToolOutput::err(format!(
                "Blocked in plan mode: this bash command modifies state ({reason}). \
                 Plan mode is read-only. To make changes, switch to act mode (Alt+Tab)."
            ));
        }
    }
    let ctx = ToolContext {
        session_id: session.id.clone(),
        message_id: tc.id.clone(),
        agent: session.agent.name.clone(),
        working_dir: session.working_dir.clone(),
        max_output: MAX_OUTPUT,
        proxy: session.config.network.proxy.clone(),
    };
    match registry.get(&tc.name) {
        Some(tool) => {
            let mut cancel_fut = std::pin::pin!(await_cancel(session));
            let exec = tool.execute(tc.input.clone(), &ctx);
            let mut deadline = std::pin::pin!(tokio::time::sleep(timeout));
            tokio::select! {
                biased;
                _ = &mut cancel_fut => ToolOutput::err("interrupted"),
                _ = &mut deadline => ToolOutput::err(format!(
                    "tool `{}` timed out after {} without producing a result",
                    tc.name, fmt_dur(timeout)
                )),
                o = exec => o.unwrap_or_else(|e| ToolOutput::err(format!("{e:#}"))),
            }
        }
        None => ToolOutput::err(format!("unknown tool: {}", tc.name)),
    }
}

/// Render a duration compactly (seconds when >= 1 s, milliseconds otherwise) so
/// the timeout message reads naturally for both the 10-minute default and the
/// sub-second durations used in tests.
fn fmt_dur(d: Duration) -> String {
    if d.as_secs() >= 1 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::SessionEvent;
    use opencoder_core::{resolve_agent, Config, Tool, ToolContext, ToolOutput};
    use opencoder_llm::{ChatStream, MockChatClient};

    /// A tool whose `execute` future never resolves, to exercise the timeout
    /// safety net without depending on a real long-running tool.
    struct HangingTool;

    #[async_trait]
    impl Tool for HangingTool {
        fn name(&self) -> &str {
            "hang"
        }
        fn description(&self) -> &str {
            "never resolves"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    /// A tool that resolves instantly, to confirm the timeout does not trip
    /// for well-behaved tools.
    struct FastTool;

    #[async_trait]
    impl Tool for FastTool {
        fn name(&self) -> &str {
            "fast"
        }
        fn description(&self) -> &str {
            "resolves immediately"
        }
        fn parameters(&self) -> serde_json::Value {
            json!({})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::ok("done"))
        }
    }

    fn make_session() -> SessionState {
        SessionState::new(
            "sess-test",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::env::temp_dir().join("opencer-execute-tests"),
        )
    }

    #[tokio::test]
    async fn hung_tool_returns_timeout_error() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("hang".to_string(), Arc::new(HangingTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-1".into(),
            name: "hang".into(),
            input: json!({}),
        };
        let out =
            execute_call_with_timeout(&tc, &session, &registry, &sink, Duration::from_millis(50))
                .await;
        assert!(out.is_error);
        assert!(
            out.content.contains("timed out"),
            "expected timeout message, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn fast_tool_is_unaffected_by_timeout() {
        let session = make_session();
        let registry: HashMap<String, ToolArc> =
            [("fast".to_string(), Arc::new(FastTool) as ToolArc)]
                .into_iter()
                .collect();
        let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
        let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
        let tc = CompletedToolCall {
            id: "tc-2".into(),
            name: "fast".into(),
            input: json!({}),
        };
        // A short timeout that would trip if the tool hung; a fast tool must
        // still return its real result, not the timeout error.
        let out =
            execute_call_with_timeout(&tc, &session, &registry, &sink, Duration::from_secs(30))
                .await;
        assert!(!out.is_error);
        assert_eq!(out.content, "done");
    }
}
