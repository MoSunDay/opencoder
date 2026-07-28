use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencoder_core::{MessageUsage, ToolArc};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};

use crate::prompt::build_system;
use crate::tools::schema_for;
use crate::SessionState;

use super::event::SessionEvent;
use super::steer::{await_cancel, await_turn_cancel};

pub(super) async fn run_one_llm_call(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<(String, String, Vec<CompletedToolCall>, Option<Usage>)> {
    let system = build_system(
        &session.agent,
        &session.working_dir,
        session.skill_prompt_cloned().as_deref(),
        &session.config.capabilities,
    );
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    let openai_msgs = lower_messages(&to_send);

    let skill_body = session.skill_prompt_cloned();
    let unlocked = crate::tools::latent::unlocked_from_body(skill_body.as_deref());
    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| {
            session.agent.tools.allows(name)
                && session.config.capabilities.tool_enabled(name)
                && (!crate::tools::latent::is_latent_tool(name.as_str())
                    || unlocked.contains(name.as_str()))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed, session.agent.kind, &session.config.capabilities);

    let req = ChatRequest {
        model: session.model.clone(),
        messages: openai_msgs,
        tools: tool_schemas,
        tool_choice: if allowed.is_empty() {
            None
        } else {
            Some("auto".into())
        },
        temperature: None,
        max_tokens: session.config.max_tokens,
        reasoning_effort: session.config.reasoning_effort.clone(),
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut completed: Option<(String, Vec<CompletedToolCall>, Option<Usage>)> = None;
    let mut reasoning_buf = String::new();
    // True once a `Retrying` status has been shown; cleared (with an empty
    // Status event) the moment real content streams so the "↻ retry" badge
    // doesn't linger after recovery.
    let mut retried = false;
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    let mut turn_cancel_fut = std::pin::pin!(await_turn_cancel(session));
    let idle_dur = session.config.stream_idle_timeout();
    loop {
        // Recreated each iteration so every received event resets the idle
        // window: a stalled stream (no events for `idle_dur`) trips the guard
        // below. SSE keep-alive comments carry no content and never reach this
        // channel, so a connection dribbling only keep-alives is treated as idle.
        let mut idle = std::pin::pin!(tokio::time::sleep(idle_dur));
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            _ = &mut turn_cancel_fut => {
                // Turn interrupted by subagent steer "submit-now": return an
                // empty turn. The caller (run_loop) detects this via
                // is_turn_cancelled and continues the loop to absorb pending
                // steers — it does NOT break like a real cancel.
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            _ = &mut idle => {
                on_event(SessionEvent::Status("stream idle".into()));
                return Err(anyhow!(
                    "stream idle timeout: no events received in {:?} — the upstream may be stalled or sending keep-alive without content",
                    idle_dur
                ));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        on_event(SessionEvent::TextDelta(t));
                    }
                    LlmEvent::ReasoningDelta(r) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        reasoning_buf.push_str(&r);
                        on_event(SessionEvent::ReasoningDelta(r));
                    }
                    LlmEvent::ToolCallStart { .. } | LlmEvent::ToolCallDelta { .. } => {}
                    LlmEvent::Completed { text, tool_calls, usage } => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        completed = Some((text, tool_calls, usage));
                    }
                    LlmEvent::Retrying { attempt, max } => {
                        retried = true;
                        on_event(SessionEvent::Status(format!(
                            "\u{21bb} retry {attempt}/{max}"
                        )));
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                }
            }
        }
    }
    let (text, tool_calls, usage) =
        completed.ok_or_else(|| anyhow!("stream ended without completion"))?;
    Ok((text, reasoning_buf, tool_calls, usage))
}

pub(super) fn core_usage(u: &Usage) -> MessageUsage {
    MessageUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        cache_read_tokens: u.cache_read_tokens,
        cache_creation_tokens: u.cache_creation_tokens,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use tokio::sync::mpsc;

    use opencoder_core::{resolve_agent, Config, ToolArc};
    use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};

    use crate::SessionState;

    use super::run_one_llm_call;

    /// A chat stream that opens a channel but never sends any event. Simulates
    /// a stalled upstream that keeps the connection alive (so the HTTP
    /// read_timeout never trips) but delivers no content — the exact scenario
    /// the idle watchdog is designed to catch.
    struct StalledClient;

    impl ChatStream for StalledClient {
        fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
            let (tx, rx) = mpsc::channel::<LlmEvent>(128);
            // Leak the sender so it is never dropped — the channel stays open
            // forever but no events are ever sent. This faithfully simulates a
            // stalled upstream that keeps the connection alive but delivers no
            // content.
            std::mem::forget(tx);
            Ok(rx)
        }
        fn backend(&self) -> &'static str {
            "stalled"
        }
    }

    fn make_session(client: Arc<dyn ChatStream>, idle_secs: u64) -> SessionState {
        let cfg = Config {
            stream_idle_timeout_secs: Some(idle_secs),
            ..Default::default()
        };
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            cfg,
            client,
            std::env::temp_dir().join("opencoder-idle-test"),
        )
    }

    #[tokio::test]
    async fn idle_stream_triggers_timeout() {
        let session = make_session(Arc::new(StalledClient) as Arc<dyn ChatStream>, 1);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let result = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        assert!(result.is_err(), "expected error, got: {:?}", result);
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("idle timeout"),
            "expected idle timeout error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn busy_stream_unaffected_by_idle_timeout() {
        let mock = MockChatClient::new().push_script(vec![
            LlmEvent::TextDelta("hello".into()),
            LlmEvent::Completed {
                text: "hello".into(),
                tool_calls: vec![],
                usage: None,
            },
        ]);
        let session = make_session(Arc::new(mock) as Arc<dyn ChatStream>, 30);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let result = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, _, _, _) = result.unwrap();
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    async fn idle_timeout_does_not_fire_if_events_keep_coming() {
        // Events arriving within the idle window must not trigger the guard,
        // even if the total stream duration exceeds the idle duration.
        let mock = MockChatClient::new().push_script(vec![
            LlmEvent::TextDelta("a".into()),
            LlmEvent::TextDelta("b".into()),
            LlmEvent::TextDelta("c".into()),
            LlmEvent::Completed {
                text: "abc".into(),
                tool_calls: vec![],
                usage: None,
            },
        ]);
        // Short idle window — would trip if the guard weren't reset on each event.
        let session = make_session(Arc::new(mock) as Arc<dyn ChatStream>, 2);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let result = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, _, _, _) = result.unwrap();
        assert_eq!(text, "abc");
    }
}
