//! Shadow VERIFY sub-agent: an isolated one-shot judgement that NEVER touches
//! the main transcript.
//!
//! This is the same pattern `generate_title_inner` / compaction already use
//! (`lower_messages(snapshot) -> ChatRequest -> chat_stream -> collect`), but
//! explicitly never calls [`SessionState::record`]. The snapshot is built in
//! memory, sent once, parsed, and dropped — so the judgement exchange never
//! pollutes `session.messages` and cannot affect subsequent turns.

use std::sync::Arc;

use anyhow::Result;
use opencoder_core::{ContentBlock, Message};
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent};

use crate::autopilot::decision::parse_verdict;
use crate::autopilot::prompts::{verify_system_prompt, verify_user_prompt};
use crate::autopilot::state::{ApState, VerifyVerdict};
use crate::dangling_tools::{tool_use_ids, tool_use_ids_without_result};
use crate::runner::new_id;
use crate::SessionState;

/// Headroom reserved out of the context window for the judge system prompt, the
/// goal question and the model's output tokens. The remaining budget is what
/// the cloned transcript may occupy.
const VERIFY_RESERVED_TOKENS: u64 = 2_000;
/// Per-message structural overhead, matching `opencoder_llm::estimate_messages`.
const MSG_OVERHEAD: usize = 4;

/// Run up to `retries` isolated one-shot VERIFY calls against a throwaway
/// snapshot of the current transcript. Returns the first parseable verdict, or
/// [`VerifyVerdict::Malformed`] if none parse.
///
/// `session.messages` is read-only here (immutable borrow) — nothing is
/// recorded or persisted. A transient LLM error counts as a malformed attempt
/// and is retried within the budget.
pub async fn verify(session: &SessionState, state: &ApState, retries: u32) -> VerifyVerdict {
    let msgs = lower_messages(&build_snapshot(session, state));

    for _ in 0..retries {
        let req = ChatRequest {
            model: session.config.small_model_or_primary().to_string(),
            messages: msgs.clone(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: Some(0.0),
            max_tokens: Some(8),
            reasoning_effort: None,
            cache_salt: None,
        };
        match drain_one_shot(&session.client, req).await {
            Ok(text) => match parse_verdict(&text) {
                // Affirmative ("yes") → the goal is fully achieved.
                Some(true) => return VerifyVerdict::Complete,
                // Negative ("no") → more work is still needed.
                Some(false) => return VerifyVerdict::MoreWork,
                None => continue, // malformed → retry within budget
            },
            Err(_) => continue, // transient error → retry within budget
        }
    }
    VerifyVerdict::Malformed
}

/// Build the ephemeral snapshot: judge system prompt + a truncated clone of the
/// transcript + the goal question.
///
/// The transcript is capped to the most recent messages that fit
/// `context_limit - VERIFY_RESERVED_TOKENS` (estimated tokens), so a long
/// autopilot run never overflows the small model's window. The goal is
/// re-stated verbatim in the question, so dropping old turns never loses the
/// anchor the judge is measured against.
fn build_snapshot(session: &SessionState, state: &ApState) -> Vec<Message> {
    let budget = session
        .config
        .context_limit()
        .saturating_sub(VERIFY_RESERVED_TOKENS) as usize;
    let mut snapshot = Vec::with_capacity(session.messages.len() + 2);
    snapshot.push(Message::system(new_id(), verify_system_prompt()));
    if opencoder_llm::estimate_messages(&session.messages) <= budget {
        snapshot.extend(session.messages.iter().cloned());
    } else {
        // Sliding window: walk from the most recent message backward, keeping
        // as many as fit the budget, then restore original order.
        let mut kept: Vec<Message> = Vec::new();
        let mut cost = 0usize;
        for m in session.messages.iter().rev() {
            let msg_cost = opencoder_llm::estimate(&m.estimate_chars()) + MSG_OVERHEAD;
            if cost + msg_cost > budget {
                break;
            }
            cost += msg_cost;
            kept.push(m.clone());
        }
        kept.reverse();
        // The window boundary can split tool_use/tool_result pairs: a leading
        // `tool_result` whose `tool_use` fell outside the window, or a
        // trailing unanswered `tool_use`. OpenAI-compatible providers reject
        // both with HTTP 400 — which would fail every VERIFY retry and
        // degrade the verdict to Malformed. Repair the edges (pure, on the
        // already-cloned messages) before extending the snapshot.
        let kept = repair_window_pairing(kept);
        snapshot.extend(kept);
    }
    snapshot.push(Message::user(new_id(), verify_user_prompt(&state.goal)));
    snapshot
}

/// Repair tool_use/tool_result pairing at the edges of a windowed transcript
/// slice so the judge request stays well-formed for OpenAI-compatible
/// providers. Pure: consumes and returns owned messages — `session` is never
/// touched.
///
/// Two defects a sliding-window cut can introduce (both are hard 400s):
///
/// - a leading `tool_result` whose `tool_use` assistant message fell outside
///   the window → the whole leading run of orphan-result messages is dropped;
/// - a trailing `tool_use` with no `tool_result` inside the window → those
///   blocks are stripped from the tail message (dropping it if nothing with
///   content remains), repeating on the new tail.
///
/// Runs to a fixpoint because each pass strictly shrinks the slice (messages
/// or blocks); the iteration cap is a belt-and-braces bound, not an expected
/// trip count.
fn repair_window_pairing(kept: Vec<Message>) -> Vec<Message> {
    let footprint = |ms: &[Message]| ms.len() + ms.iter().map(|m| m.blocks.len()).sum::<usize>();
    let mut msgs = kept;
    for _ in 0..=footprint(&msgs) {
        let before = footprint(&msgs);
        msgs = drop_leading_orphan_results(msgs);
        msgs = strip_trailing_unanswered_tool_uses(msgs);
        if footprint(&msgs) == before {
            break; // stable: neither edge needed repair this pass
        }
    }
    msgs
}

/// Drop the leading run of messages that carry `ToolResult` blocks whose
/// `tool_use` is not present inside `msgs` (their assistant message fell
/// outside the window, so the results are unanswerable orphans). Stops at the
/// first head message that is pair-clean. Pure.
fn drop_leading_orphan_results(kept: Vec<Message>) -> Vec<Message> {
    let mut msgs = kept;
    while !msgs.is_empty() {
        let uses = tool_use_ids(&msgs);
        let head_is_orphan = msgs[0].blocks.iter().any(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => !uses.contains(tool_use_id),
            _ => false,
        });
        if !head_is_orphan {
            break;
        }
        msgs.remove(0);
    }
    msgs
}

/// Strip unanswered `tool_use` blocks from the tail message. If the stripped
/// message still has content it is kept (as a clone with the dangling calls
/// removed); if it becomes empty it is dropped and the check repeats on the
/// new tail. Pure — the input slice is consumed, never mutated in place.
fn strip_trailing_unanswered_tool_uses(kept: Vec<Message>) -> Vec<Message> {
    let mut msgs = kept;
    while let Some(tail) = msgs.last() {
        let dangling = tool_use_ids_without_result(&msgs);
        let carries_dangling = tail.blocks.iter().any(|b| match b {
            ContentBlock::ToolUse { id, .. } => dangling.contains(id),
            _ => false,
        });
        if !carries_dangling {
            break;
        }
        let stripped: Vec<ContentBlock> = tail
            .blocks
            .iter()
            .filter(|b| match b {
                ContentBlock::ToolUse { id, .. } => !dangling.contains(id),
                _ => true,
            })
            .cloned()
            .collect();
        let old_tail = msgs.pop().expect("tail checked above");
        if has_content(&stripped) {
            msgs.push(Message {
                blocks: stripped,
                ..old_tail
            });
            // Every dangling use in this message was stripped, so the tail is
            // now pair-clean — no need for another pass.
            break;
        }
        // Empty after stripping → dropped; loop re-examines the new tail.
    }
    msgs
}

/// Whether a block list still carries anything a provider would accept as a
/// message body: any non-empty text/reasoning, or any structured block
/// (image / answered tool_use / tool_result). Used to decide whether a
/// stripped tail message is worth keeping.
fn has_content(blocks: &[ContentBlock]) -> bool {
    blocks.iter().any(|b| match b {
        ContentBlock::Text { text } | ContentBlock::Reasoning { text } => !text.trim().is_empty(),
        _ => true,
    })
}

/// Collect a single completion into a String. Mirrors the sink loop in
/// `generate_title_inner`: accumulate deltas, snap to the terminal `Completed`
/// text, surface stream errors. Returns the final assistant text (possibly
/// empty).
async fn drain_one_shot(client: &Arc<dyn ChatStream>, req: ChatRequest) -> Result<String> {
    let mut rx = client.chat_stream(req)?;
    let mut text = String::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            LlmEvent::TextDelta(t) => text.push_str(&t),
            LlmEvent::Completed { text: t, .. } => {
                if !t.is_empty() {
                    text = t;
                }
                break;
            }
            LlmEvent::Retrying { .. } => {
                // Mid-stream retry: drop deltas so the two attempts aren't
                // concatenated; the final `Completed` overwrites `text`.
                text.clear();
            }
            LlmEvent::Error(e) => return Err(anyhow::anyhow!(e)),
            _ => {}
        }
    }
    Ok(text)
}

// ── tests: build_snapshot window pairing ─────────────────────────────────
//
// `build_snapshot` is pure, so these construct throwaway `SessionState`s
// (no store, mock client — nothing is ever streamed) and assert on the
// returned snapshot only. Budget = context_limit - VERIFY_RESERVED_TOKENS;
// small explicit `context_limit`s force the sliding-window branch with
// hand-sized messages.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dangling_tools::tool_result_ids;
    use opencoder_core::message::MessageUsage;
    use opencoder_core::{resolve_agent, Config, Role};
    use opencoder_llm::{ChatStream, MockChatClient};
    use std::collections::HashSet;

    fn tool_use_msg(id: &str, use_id: &str) -> Message {
        Message {
            id: id.into(),
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolUse {
                id: use_id.into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "true"}),
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        }
    }

    fn tool_result_msg(id: &str, use_id: &str) -> Message {
        Message {
            id: id.into(),
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: use_id.into(),
                content: "ok".into(),
                is_error: false,
                images: Vec::new(),
            }],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        }
    }

    fn session_with(context_limit: Option<u64>, messages: Vec<Message>) -> SessionState {
        let agent = resolve_agent("act").expect("act agent registered");
        let config = Config {
            context_limit,
            ..Config::default()
        };
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        let mut session = SessionState::new(
            "verify-snapshot-test",
            agent,
            config,
            client,
            std::env::temp_dir(),
        );
        session.messages = messages;
        session
    }

    /// Result ids with no `tool_use` in the slice — mirror of the shared
    /// `tool_use_ids_without_result` helper.
    fn orphan_result_ids(messages: &[Message]) -> HashSet<String> {
        let uses = tool_use_ids(messages);
        tool_result_ids(messages)
            .into_iter()
            .filter(|id| !uses.contains(id))
            .collect()
    }

    /// Well-formedness predicate: every `tool_use` id is answered by a
    /// `tool_result` AND every `tool_result` references a `tool_use` present
    /// in the slice — exactly what an OpenAI-compatible provider demands.
    fn pairs_are_intact(messages: &[Message]) -> bool {
        tool_use_ids_without_result(messages).is_empty() && orphan_result_ids(messages).is_empty()
    }

    fn ids_of(messages: &[Message]) -> Vec<&str> {
        messages.iter().map(|m| m.id.as_str()).collect()
    }

    fn block_counts(messages: &[Message]) -> Vec<usize> {
        messages.iter().map(|m| m.blocks.len()).collect()
    }

    #[test]
    fn window_drops_leading_orphan_tool_result() {
        // Budget = 2011 - 2000 = 11 tokens. Walking backward from the tail the
        // user (5) and the tool_result (5) fit, the fat assistant tool_use
        // message does not — so the window opens ON the result whose use fell
        // outside: the exact orphan the repair must drop.
        let msgs = vec![
            Message::user("m0", "x".repeat(200)), // ~54 tokens, always outside
            {
                let mut m = tool_use_msg("m1", "tu1");
                m.blocks.insert(
                    0,
                    ContentBlock::text("x".repeat(200)), // ~60 tokens: the breaker
                );
                m
            },
            tool_result_msg("m2", "tu1"),
            Message::user("m3", "hi"),
        ];
        let session = session_with(Some(2011), msgs.clone());
        let snapshot = build_snapshot(&session, &ApState::new("goal".into()));
        assert!(pairs_are_intact(&snapshot), "snapshot pairs must be intact");
        assert!(orphan_result_ids(&snapshot).is_empty());
        assert!(
            snapshot.iter().all(|m| m.id != "m2"),
            "orphan tool_result message m2 must not survive the window"
        );
        assert_eq!(snapshot.len(), 3, "system + m3 + question only");
        assert_eq!(snapshot[1].id, "m3");
        // Purity: the session transcript is untouched (ids + block counts).
        assert_eq!(ids_of(&session.messages), ids_of(&msgs));
        assert_eq!(block_counts(&session.messages), block_counts(&msgs));
    }

    #[test]
    fn window_strips_trailing_unanswered_tool_use_blocks() {
        // The transcript ends mid-tool-batch (hard interrupt): the assistant
        // message carries text + an unanswered tool_use. Kept by the window,
        // its dangling call must be stripped while the text survives.
        let tail = {
            let mut m = tool_use_msg("m2", "tu9");
            m.blocks
                .insert(0, ContentBlock::text("running the build now"));
            m
        };
        let session = session_with(
            Some(2048), // budget 48: [m1(5), m2(~15)] kept, pad(~104) outside
            vec![
                Message::user("m0", "x".repeat(400)),
                Message::user("m1", "hi"),
                tail,
            ],
        );
        let snapshot = build_snapshot(&session, &ApState::new("goal".into()));
        assert!(pairs_are_intact(&snapshot));
        assert!(
            tool_use_ids(&snapshot).is_empty(),
            "unanswered tu9 must be stripped"
        );
        let kept_tail = snapshot
            .iter()
            .find(|m| m.id == "m2")
            .expect("tail message survives with its text");
        assert_eq!(kept_tail.blocks.len(), 1);
        assert_eq!(kept_tail.blocks[0].as_text(), Some("running the build now"));
    }

    #[test]
    fn window_drops_tail_message_that_is_only_an_unanswered_tool_use() {
        // Stripping every block empties the message → it must be dropped.
        let session = session_with(
            Some(2048),
            vec![
                Message::user("m0", "x".repeat(400)),
                Message::user("m1", "hi"),
                tool_use_msg("m2", "tu9"),
            ],
        );
        let snapshot = build_snapshot(&session, &ApState::new("goal".into()));
        assert!(pairs_are_intact(&snapshot));
        assert_eq!(snapshot.len(), 3, "system + m1 + question only");
        assert!(snapshot.iter().all(|m| m.id != "m2"));
        assert_eq!(snapshot[1].id, "m1");
        assert_eq!(snapshot[2].role, Role::User);
    }

    #[test]
    fn window_strips_a_run_of_dangling_tool_call_tails() {
        // Two consecutive unanswered tool-call carriers at the tail: the strip
        // must repeat until the tail is pair-clean.
        let session = session_with(
            Some(2048),
            vec![
                Message::user("m0", "x".repeat(400)),
                Message::user("m1", "hi"),
                tool_use_msg("m2", "tu8"),
                tool_use_msg("m3", "tu9"),
            ],
        );
        let snapshot = build_snapshot(&session, &ApState::new("goal".into()));
        assert!(pairs_are_intact(&snapshot));
        assert!(tool_use_ids(&snapshot).is_empty());
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[1].id, "m1");
    }

    #[test]
    fn small_transcript_fast_path_is_unchanged() {
        let msgs = vec![
            Message::user("u1", "do the thing"),
            {
                let mut m = tool_use_msg("a1", "tu1");
                m.blocks.insert(0, ContentBlock::text("working"));
                m
            },
            tool_result_msg("r1", "tu1"),
            Message::user("u2", "continue"),
        ];
        let session = session_with(None, msgs.clone()); // default 128k limit → fast path
        let snapshot = build_snapshot(&session, &ApState::new("do the thing".into()));
        assert_eq!(snapshot.len(), msgs.len() + 2);
        assert_eq!(snapshot[0].role, Role::System);
        // The transcript is cloned verbatim, in order, between the two
        // synthesized messages.
        assert_eq!(ids_of(&snapshot[1..msgs.len() + 1]), ids_of(&msgs));
        for (kept, orig) in snapshot[1..msgs.len() + 1].iter().zip(msgs.iter()) {
            assert_eq!(kept.role, orig.role);
            assert_eq!(kept.blocks.len(), orig.blocks.len());
        }
        assert_eq!(snapshot.last().unwrap().role, Role::User);
        assert!(pairs_are_intact(&snapshot));
    }

    #[test]
    fn many_pairs_straddling_the_boundary_stay_well_formed() {
        // Property: pad + 8 use/result pairs + tail user; budget 48 keeps only
        // the last ~3 pairs, so the boundary cuts inside the pair run.
        let mut msgs = vec![Message::user("pad", "x".repeat(400))];
        for i in 0..8 {
            msgs.push(tool_use_msg(&format!("a{i}"), &format!("tu{i}")));
            msgs.push(tool_result_msg(&format!("t{i}"), &format!("tu{i}")));
        }
        msgs.push(Message::user("tail", "done"));
        let session = session_with(Some(2048), msgs.clone());
        let snapshot = build_snapshot(&session, &ApState::new("goal".into()));
        // The window actually truncated (branch taken, pairs retained).
        assert!(snapshot.len() > 3, "window must keep some pairs");
        assert!(!tool_use_ids(&snapshot).is_empty(), "non-vacuous predicate");
        assert!(
            pairs_are_intact(&snapshot),
            "every use answered and every result matched: uses={:?} orphans={:?}",
            tool_use_ids_without_result(&snapshot),
            orphan_result_ids(&snapshot)
        );
        // Purity: the full transcript is untouched.
        assert_eq!(session.messages.len(), msgs.len());
        assert_eq!(session.messages.first().unwrap().id, "pad");
        assert_eq!(session.messages.last().unwrap().id, "tail");
    }
}
