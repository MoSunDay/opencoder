//! Replay logic for reconstructing a [`ChatView`] from persisted messages and
//! subagent task records during session resume or context restore.
//!
//! Extracted from `session_ui.rs` to keep that module focused on UI-state
//! snapshot/restore. The public entry point is [`replay_into_chat`].

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_llm::estimate_messages_for_display;
use opencoder_session::SessionEvent;
use opencoder_store::{Store, SubagentStatus, SubagentTaskRecord};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::chat::{
    coalesce_steps, short, single_step_group, summarize, tool_output_lines, ChatBlock, ChatView,
    Step, ToolCall,
};
use crate::terminal_text::{sanitize_multiline, sanitize_single_line};
use crate::theme;

/// Emit one replay segment as a ladder: pre-Say reasoning plus the segment's
/// calls form the turn's StepGroup (a call-less step when the turn only
/// thought before speaking). Mirrors the live path's grouping rules; pure
/// w.r.t. the passed buffers (drains both on flush).
fn flush_segment(
    chat: &mut ChatView,
    seg_thinking: &mut Vec<String>,
    seg_calls: &mut Vec<ToolCall>,
) {
    if seg_thinking.is_empty() && seg_calls.is_empty() {
        return;
    }
    let thinking_raw = seg_thinking.join("");
    seg_thinking.clear();
    // Render eagerly, mirroring the live path (thinking is rendered markdown
    // the moment it is absorbed into a step): replayed steps carry the
    // rendered body too, so `.thinking` readers (disclosure, copy mode) see
    // it without waiting for a lazy render pass that replay never runs.
    let thinking = crate::markdown::render(&thinking_raw);
    let steps = vec![Step {
        thinking_raw,
        thinking,
        thinking_dirty: false,
        calls: std::mem::take(seg_calls),
        open: false,
        calls_open: false,
        sealed: true,
    }];
    chat.blocks.push(ChatBlock::StepGroup {
        steps,
        open: false,
        progress_active: false,
    });
}

/// Replay a single persisted message into `chat`: reconstruct `Assistant` text
/// and `ChatBlock::StepGroup` blocks (headers from `ToolUse`, outputs from
/// matching `ToolResult`s), mirroring the live `ChatView::apply` path
/// (calls accumulate in the trailing step until new Thinking) for resumed/compacted
/// sessions. Thinking folding into steps happens once, via `coalesce_steps`
/// at the end of replay.
/// Summary text of a synthetic compaction-summary message, if `msg` is one.
/// Mirrors the session-side `SUMMARY_PREFIX` marker (single source of truth:
/// `opencoder_session::compaction`), which the resume path also uses to
/// rebuild the head of a compacted transcript.
fn compaction_summary_of(msg: &Message) -> Option<String> {
    if !msg.synthetic {
        return None;
    }
    let text = msg.text();
    text.strip_prefix(opencoder_session::compaction::SUMMARY_PREFIX)
        .and_then(|rest| rest.strip_prefix('\n'))
        .map(str::to_string)
}

/// Render a compaction summary as the same collapsible Compaction block the
/// live path finalizes (collapsed, not streaming).
///
/// The folded transcript is `[summary, tail...]` — summary first, the
/// chronologically correct order for the LLM context (`runner/llm_call.rs`
/// sends the list as-is). The UI order is deliberately different: the block
/// marks WHERE the history was folded, so it renders AFTER the retained tail
/// turns and BEFORE the run that continues below it (`tail + summary +
/// continued execution`). Replay entry points therefore collect summaries
/// during the message loop and push them once the regular blocks (and the
/// step fold) are complete.
fn push_compaction_summary_block(chat: &mut ChatView, summary: &str) {
    chat.blocks.push(ChatBlock::Compaction {
        text: sanitize_multiline(summary).into_owned(),
        collapsed: true,
        streaming: false,
    });
}

/// Verbatim echo text a user message replays as: `display` is the single
/// source of truth; recorded blocks are the legacy fallback. Synthetic
/// messages without a display are internal markers and never echo.
fn user_echo_text(msg: &Message) -> Option<String> {
    if msg.role != Role::User || (msg.synthetic && msg.display.is_none()) {
        return None;
    }
    let text = match &msg.display {
        Some(d) => d.clone(),
        None => msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    };
    let text = sanitize_multiline(&text).into_owned();
    (!text.is_empty()).then_some(text)
}

pub(super) fn replay_one(
    chat: &mut ChatView,
    msg: &Message,
    prefetched: &HashMap<String, Vec<u8>>,
) {
    match msg.role {
        Role::User => {
            // Synthetic user messages (plan->act handoff, compaction summaries)
            // are internal — skip `user:` blocks on replay. Skill triggers are
            // the exception: they carry the verbatim input as `display`, which
            // IS rendered. Steer/queue promotions are real user input and ARE
            // rendered (via `display`, tokens included) so the user sees their
            // prompts verbatim after resume. Compaction summaries are
            // intercepted by the replay entry points BEFORE `replay_one`
            // runs (see `push_compaction_summary_block`): they render as a
            // deferred Compaction block, never in-place here.
            if msg.synthetic && msg.display.is_none() {
                return;
            }
            // Echo contract: `display` is the verbatim input single source of
            // truth; fall back to the recorded blocks for legacy rows.
            let text: String = match &msg.display {
                Some(d) => d.clone(),
                None => msg
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            let text = sanitize_multiline(&text).into_owned();
            if chat.first_prompt.is_none() {
                let t = text.trim();
                if !t.is_empty() {
                    chat.first_prompt = Some(t.to_string());
                }
            }
            let has_images = msg.blocks.iter().any(|b| b.as_image().is_some());
            if text.is_empty() && !has_images {
                return;
            }
            let rendered = crate::markdown::render(&text);
            if !rendered.is_empty() {
                chat.blocks.push(ChatBlock::User { rendered });
            }
            // Render any Image blocks inline after the text. Remote URLs are
            // resolved from the prefetched-bytes map (async-fetched above).
            for b in &msg.blocks {
                if let ContentBlock::Image { url, .. } = b {
                    let filename = sanitize_single_line(&crate::image_util::extract_filename(url))
                        .into_owned();
                    let rendered_img = crate::image_render::render_image_from_url(url, prefetched);
                    chat.blocks.push(ChatBlock::Image {
                        filename,
                        rendered: rendered_img,
                    });
                }
            }
            chat.push_marker(Line::from(""));
        }
        Role::Assistant => {
            // Session-lifetime token cost: sum the persisted per-message
            // usage (mirrors the live LlmUsage accumulation path).
            chat.tokens_total = chat.tokens_total.saturating_add(msg.usage.total_tokens);
            // Provider-truth context: keep the most recent non-zero
            // `total_tokens` (mirrors the live per-round overwrite). Later
            // messages win, so a compaction-truncated replay naturally
            // reflects the surviving tail of the transcript.
            if msg.usage.total_tokens > 0 {
                chat.real_context_tokens = Some(msg.usage.total_tokens);
            }
            // Rebuild in BLOCK ORDER, mirroring the live path's Turn
            // contract: a Text block (Say) CLOSES a Turn — reasoning/tool
            // blocks that follow it belong to the NEXT turn's ladder, not
            // the one above the Say. Segments between Says accumulate
            // exactly like live rounds within one turn. `coalesce_steps`
            // later merges call-only steps that share a turn.
            let mut seg_thinking: Vec<String> = Vec::new();
            let mut seg_calls: Vec<ToolCall> = Vec::new();
            for b in &msg.blocks {
                match b {
                    ContentBlock::Reasoning { text } => {
                        seg_thinking.push(sanitize_multiline(text).into_owned());
                    }
                    ContentBlock::Text { text } if !text.trim().is_empty() => {
                        flush_segment(chat, &mut seg_thinking, &mut seg_calls);
                        let raw = sanitize_multiline(text).into_owned();
                        chat.blocks.push(ChatBlock::Assistant {
                            raw,
                            rendered: crate::markdown::render(text),
                            done: true,
                        });
                    }
                    ContentBlock::ToolUse { id, name, input } if name != "task" => {
                        seg_calls.push(ToolCall {
                            id: id.clone(),
                            header: Line::from(vec![
                                Span::styled(
                                    format!("\u{25b8} {} ", sanitize_single_line(name)),
                                    Style::default()
                                        .fg(theme::accent())
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(summarize(input), Style::default().fg(theme::muted())),
                            ]),
                            output: Vec::new(),
                            // Replayed calls carry no wall-clock timing: mark them
                            // finished (elapsed 0) so no epoch-scale live timer or
                            // "running" hint renders on resume.
                            started_at_ms: Some(0),
                            elapsed_ms: Some(0),
                            expanded: false,
                        });
                    }
                    _ => {}
                }
            }
            // Trailing segment after the last Say (or the whole message
            // when it never spoke): its ladder follows the same contract.
            flush_segment(chat, &mut seg_thinking, &mut seg_calls);
        }
        Role::Tool => {
            for b in &msg.blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    images,
                    ..
                } = b
                {
                    let color = if *is_error {
                        theme::err_color()
                    } else {
                        theme::muted()
                    };
                    let out = tool_output_lines(content, color);
                    let target = chat.blocks.iter().enumerate().rev().find_map(|(gi, blk)| {
                        if let ChatBlock::StepGroup { steps, .. } = blk {
                            steps
                                .iter()
                                .enumerate()
                                .rev()
                                .find_map(|(si, s)| {
                                    s.calls
                                        .iter()
                                        .rposition(|c| c.id == *tool_use_id)
                                        .map(|ci| (si, ci))
                                })
                                .map(|(si, ci)| (gi, si, ci))
                        } else {
                            None
                        }
                    });
                    if let Some((gi, si, ci)) = target {
                        if let ChatBlock::StepGroup { steps, .. } = &mut chat.blocks[gi] {
                            steps[si].calls[ci].output.extend(out);
                        }
                    } else {
                        // Skip fallback for "task" tools — their output is
                        // shown via the Subagent block, not a StepGroup.
                        let has_subagent = chat.blocks.iter().any(|b| {
                            matches!(b, ChatBlock::Subagent { id: bid, .. } if bid == tool_use_id)
                        });
                        if !has_subagent {
                            chat.blocks.push(single_step_group(
                                ToolCall {
                                    id: tool_use_id.clone(),
                                    // Orphan ToolResult: the matching ToolUse
                                    // (name/input) never replayed, so only
                                    // the output survives to render.
                                    header: Line::from(Span::styled(
                                        "\u{25b8} (output)",
                                        Style::default().fg(theme::accent()),
                                    )),
                                    output: out,
                                    started_at_ms: None,
                                    elapsed_ms: Some(0),
                                    expanded: false,
                                },
                                Vec::new(),
                            ));
                        }
                    }
                    // Render tool-returned images inline after the text output.
                    for url in images {
                        let filename =
                            sanitize_single_line(&crate::image_util::extract_filename(url))
                                .into_owned();
                        let rendered_img =
                            crate::image_render::render_image_from_url(url, prefetched);
                        chat.blocks.push(ChatBlock::Image {
                            filename,
                            rendered: rendered_img,
                        });
                    }
                }
            }
        }
        Role::System => {}
    }
}

/// Build a fresh `ChatView` for a resumed session by replaying stored messages
/// and reconstructing subagent blocks from persisted `subagent_tasks` records.
pub async fn replay_into_chat(
    agent_name: &str,
    messages: &[Message],
    store: &Arc<dyn Store>,
    session_id: &str,
    preserve_tokens_total: u64,
) -> ChatView {
    replay_into_chat_inner(
        agent_name,
        messages,
        store,
        session_id,
        preserve_tokens_total,
        None,
    )
    .await
}

/// Replay with the in-flight turn's verbatim echo. When the folded transcript
/// already contains that prompt (mid-run compaction records the user message
/// before the runner folds the head), the replayed copy is deferred and lands
/// AFTER the Compaction marker instead of being retained in the tail AND
/// pushed again by the reset rebuild. Clear-context resets do not contain the
/// prompt yet, so the echo is simply appended below the rebuilt blocks.
async fn replay_into_chat_inner(
    agent_name: &str,
    messages: &[Message],
    store: &Arc<dyn Store>,
    session_id: &str,
    preserve_tokens_total: u64,
    deferred_echo: Option<&str>,
) -> ChatView {
    let mut chat = ChatView {
        agent: sanitize_single_line(agent_name).into_owned(),
        ..Default::default()
    };

    // Plan→act handoff card. The clear-context boundary (/act_clear_context)
    // persists internal markers here: the blank sentinel is skipped entirely,
    // and a last-say seed marker is stripped to its preserved text so the raw
    // marker never reaches the UI (the preserved reply renders like a plan
    // card: read-only context carried across the clear).
    if let Ok(Some(meta)) = store.get_session(session_id).await {
        if let Some(plan) = meta
            .handoff_plan
            .as_deref()
            .filter(|p| !opencoder_session::is_clear_context_handoff(p))
            .map(|p| {
                if opencoder_session::is_clear_context_seed(p) {
                    opencoder_session::clear_seed_text(p)
                } else {
                    p
                }
            })
        {
            let clean_plan = sanitize_multiline(plan);
            let rendered = crate::markdown::render(&clean_plan);
            if !rendered.is_empty() {
                chat.blocks.push(ChatBlock::Plan {
                    rendered,
                    raw: clean_plan.into_owned(),
                });
            }
        }
    }

    let tasks = store
        .list_subagent_tasks(session_id)
        .await
        .unwrap_or_default();
    let mut tasks_by_parent: HashMap<String, Vec<SubagentTaskRecord>> = HashMap::new();
    let mut orphan_tasks: Vec<SubagentTaskRecord> = Vec::new();
    for task in tasks {
        match &task.parent_message_id {
            Some(mid) => {
                tasks_by_parent.entry(mid.clone()).or_default().push(task);
            }
            None => {
                orphan_tasks.push(task);
            }
        }
    }
    for group in tasks_by_parent.values_mut() {
        group.sort_by_key(|t| t.started_at);
    }
    orphan_tasks.sort_by_key(|t| t.started_at);

    // Pre-fetch HTTP image URLs so remote images render during synchronous
    // replay. Data URIs are handled inline by `render_image_from_url`.
    let prefetched = prefetch_image_bytes(messages).await;

    // The in-flight prompt must be the LAST visible user message; anything
    // earlier with the same text is ordinary history and stays where it was.
    let deferred_user_idx = deferred_echo
        .filter(|echo| !echo.trim().is_empty())
        .and_then(|echo| {
            let (idx, text) = messages
                .iter()
                .enumerate()
                .rev()
                .find_map(|(idx, msg)| user_echo_text(msg).map(|text| (idx, text)))?;
            (text.trim() == echo.trim()).then_some(idx)
        });

    let mut deferred_summaries: Vec<String> = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if Some(idx) == deferred_user_idx {
            continue;
        }
        if let Some(summary) = compaction_summary_of(msg) {
            deferred_summaries.push(summary);
            continue;
        }
        replay_one(&mut chat, msg, &prefetched);
        // Interleave child blocks under their parent assistant message.
        if msg.role == Role::Assistant {
            if let Some(task_list) = tasks_by_parent.remove(&msg.id) {
                for task in task_list {
                    let block = build_subagent_block(&task, store).await;
                    push_subagent_block(&mut chat, block);
                }
            }
        }
    }

    for task in orphan_tasks {
        let block = build_subagent_block(&task, store).await;
        push_subagent_block(&mut chat, block);
    }

    // Fold replayed blocks into the step model: trailing `Thinking` runs
    // merge into the following StepGroup's first step and adjacent groups
    // coalesce — the same shape the live streaming path produces.
    coalesce_steps(&mut chat.blocks);
    // Folded-history markers land AFTER the retained turns (step folding is
    // untouched by them); the continued run appends below.
    for summary in &deferred_summaries {
        push_compaction_summary_block(&mut chat, summary);
    }
    // The running turn's boundary belongs below the folded-history marker
    // (tail + summary + continued execution). If the prompt was part of the
    // retained tail, render it here from its persisted message (images
    // included) so it is not both retained above the marker and pushed again;
    // otherwise re-push the remembered echo for a clear-context rebuild.
    if let Some(idx) = deferred_user_idx {
        replay_one(&mut chat, &messages[idx], &prefetched);
    } else if let Some(echo) = deferred_echo.filter(|echo| !echo.trim().is_empty()) {
        let rendered = crate::markdown::render(echo);
        chat.blocks.push(ChatBlock::User { rendered });
        chat.push_marker(Line::from(""));
    }
    // Full transcript token count for ctx% (system prompt added at render).
    chat.context_used = estimate_messages_for_display(messages) as u64;
    // Compaction truncates the message list, so the usage sum above can only
    // shrink across a TranscriptReset rebuild. Floor it with the live-view
    // accumulation (passed by the caller) so `[tok cost]` never regresses.
    // Both sides include subagent spend, so the floor cannot double-count.
    chat.tokens_total = chat.tokens_total.max(preserve_tokens_total);
    chat
}

/// Push a reconstructed subagent block, folding the child's lifetime token
/// spend into the parent's `[tok cost]` — mirrors the live path where
/// `SubagentChild(LlmUsage)` events accumulate into the parent view.
fn push_subagent_block(chat: &mut ChatView, block: ChatBlock) {
    if let ChatBlock::Subagent { view, .. } = &block {
        chat.tokens_total = chat.tokens_total.saturating_add(view.tokens_total);
    }
    chat.blocks.push(block);
}

/// Rebuild the chat view after a mid-run `TranscriptReset` (compaction /
/// clear-context), carrying over cross-reset UI state: saved annotation,
/// submitted flag, first prompt, and the session-lifetime
/// token accumulation (floored via `preserve_tokens_total`).
pub async fn rebuild_after_reset(
    chat: &mut ChatView,
    msgs: &[Message],
    store: &Arc<dyn Store>,
    session_id: &str,
) {
    let agent = chat.agent.clone();
    let saved_annotation_text = chat.annotation_text.clone();
    let saved_submitted = chat.submitted;
    let saved_first_prompt = chat.first_prompt.clone();
    let saved_tokens_total = chat.tokens_total;
    let saved_turn_echo = chat.pending_turn_echo.clone();
    *chat = replay_into_chat_inner(
        &agent,
        msgs,
        store,
        session_id,
        saved_tokens_total,
        saved_turn_echo.as_deref(),
    )
    .await;
    chat.annotation_text = saved_annotation_text;
    chat.submitted = saved_submitted;
    chat.first_prompt = saved_first_prompt;
    chat.pending_turn_echo = saved_turn_echo;
    // The reset happened inside the admitted turn (e.g. the compound
    // `/act_clear_context <tail>` that triggered it), and the folded
    // transcript may not contain that turn's prompt yet — it can be recorded
    // after the reset fires. `replay_into_chat_inner` puts the remembered
    // echo below the rebuilt blocks (and de-duplicates it when the folded
    // transcript already retained it), so the running ladder keeps its user
    // boundary and is never merged into the pre-reset turn.
    // Reliable completion repair must never target pre-reset blocks.
    chat.turn_block_start = chat.blocks.len();
}

/// Reconstruct a `ChatBlock::Subagent` from a persisted `SubagentTaskRecord`,
/// including rebuilding the child `ChatView` from stored events.
pub(super) async fn build_subagent_block(
    task: &SubagentTaskRecord,
    store: &Arc<dyn Store>,
) -> ChatBlock {
    let (done, ok, cancelled, summary) = match task.status {
        SubagentStatus::Completed => (
            true,
            task.ok.unwrap_or(true),
            false,
            task.result.clone().unwrap_or_default(),
        ),
        SubagentStatus::Failed => (true, false, false, task.result.clone().unwrap_or_default()),
        SubagentStatus::Cancelled => (true, false, true, "(cancelled)".to_string()),
        SubagentStatus::Running | SubagentStatus::Unknown => {
            // Interrupted during resume — display as done/failed with a marker.
            (true, false, false, "(interrupted)".to_string())
        }
    };

    let mut view = reconstruct_child_view(&task.child_session_id, &task.agent, store).await;
    // The persisted event log may end mid-stream (crash / kill between
    // flushes): the child never emitted its round-terminal frames, and
    // unlike the live path — whose `mark_subagent_done` finalizes the child
    // view — a rebuilt view has no turn end to repair it. Finalize once
    // here so a truncated log never resurrects an open Say whose body
    // renders raw markdown.
    view.finalize_assistant();

    ChatBlock::Subagent {
        id: task.task_id.clone(),
        child_session_id: task.child_session_id.clone(),
        kind: sanitize_single_line(&task.agent).into_owned(),
        prompt: short(&task.prompt, 90),
        view,
        done,
        ok,
        cancelled,
        summary: sanitize_multiline(&summary).into_owned(),
        started_at_ms: task.started_at,
        elapsed_ms: task
            .completed_at
            .map(|c| ((c - task.started_at).max(0)) as u64),
    }
}

/// Rebuild a child `ChatView` from persisted events (primary) or messages
/// (fallback) under the child session id.
pub(super) async fn reconstruct_child_view(
    child_session_id: &str,
    agent_name: &str,
    store: &Arc<dyn Store>,
) -> ChatView {
    // Primary: replay persisted events.
    let events = store
        .events_after(child_session_id, 0)
        .await
        .unwrap_or_default();
    if !events.is_empty() {
        let mut view = ChatView {
            agent: sanitize_single_line(agent_name).into_owned(),
            ..Default::default()
        };
        for rec in &events {
            if let Some(ev) = deserialize_event(&rec.payload) {
                view.apply(&ev);
            }
        }
        // `apply` already accumulates the ctx estimate on discrete events
        // (tool start/end, queue/steer consumed, compaction, ...), so the
        // meter comes for free. Deliberately NO second full `load_messages`
        // scan per child: it doubled the query cost of every transcript
        // rebuild, and only the crash-lost tail of the event log could ever
        // be missed — a minor under-report, not a correctness bug.
        return view;
    }

    // Fallback: replay messages.
    tracing::debug!(
        child_session_id,
        "no persisted events for subagent child, falling back to messages"
    );
    let messages = store
        .load_messages(child_session_id)
        .await
        .unwrap_or_default();
    if messages.is_empty() {
        tracing::debug!(
            child_session_id,
            "no events or messages for subagent child session"
        );
    }
    replay_messages(agent_name, &messages)
}

/// Deserialize a `SessionEvent` from a stored event payload.
/// Child events are double-encoded: `Value::String(json_string)`.
pub(super) fn deserialize_event(payload: &serde_json::Value) -> Option<SessionEvent> {
    match payload {
        serde_json::Value::String(s) => serde_json::from_str::<SessionEvent>(s)
            .map_err(|e| tracing::warn!(error = %e, "failed to deserialize string event payload"))
            .ok(),
        other => serde_json::from_value::<SessionEvent>(other.clone())
            .map_err(|e| tracing::warn!(error = %e, "failed to deserialize json event payload"))
            .ok(),
    }
}

/// Overall wall-clock budget for one prefetch batch. Partial success is
/// acceptable (missing entries render a placeholder), so a single slow host
/// must never stall the whole session-switch / resume rebuild — the old
/// serial loop multiplied per-image timeouts into tens of seconds.
pub(super) const PREFETCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(8);

/// Collect and asynchronously fetch all HTTP(S) image URLs from `messages`
/// so they are available during synchronous `replay_one`. Data URIs are
/// skipped (decoded in-process). Failed fetches are silently dropped —
/// the corresponding images will show a placeholder.
pub(super) async fn prefetch_image_bytes(messages: &[Message]) -> HashMap<String, Vec<u8>> {
    prefetch_image_bytes_with(
        messages,
        |url| async move { crate::image_render::fetch_image_bytes(&url).await },
        PREFETCH_BUDGET,
    )
    .await
}

/// Budgeted, concurrent variant of [`prefetch_image_bytes`] with the fetcher
/// injected so tests can drive it without a network. Fetches run on a
/// `JoinSet`; when the overall `budget` expires the remaining tasks are
/// aborted and whatever arrived so far is returned.
pub(super) async fn prefetch_image_bytes_with<F, Fut>(
    messages: &[Message],
    fetch: F,
    budget: std::time::Duration,
) -> HashMap<String, Vec<u8>>
where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Option<Vec<u8>>> + Send + 'static,
{
    let mut urls: Vec<String> = Vec::new();
    for msg in messages {
        for b in &msg.blocks {
            if let ContentBlock::Image { url, .. } = b {
                if url.starts_with("http://") || url.starts_with("https://") {
                    urls.push(url.clone());
                }
            }
            if let ContentBlock::ToolResult { images, .. } = b {
                for url in images {
                    if url.starts_with("http://") || url.starts_with("https://") {
                        urls.push(url.clone());
                    }
                }
            }
        }
    }
    urls.sort();
    urls.dedup();

    let mut map = HashMap::new();
    if urls.is_empty() {
        return map;
    }
    let mut set = tokio::task::JoinSet::new();
    for url in urls {
        let fetch = fetch.clone();
        set.spawn(async move {
            let bytes = fetch(url.clone()).await;
            (url, bytes)
        });
    }
    let deadline = tokio::time::Instant::now() + budget;
    while let Ok(next) = tokio::time::timeout_at(deadline, set.join_next()).await {
        match next {
            // `Err(Elapsed)` can't happen here (deadline passed -> outer Err).
            Some(Ok((url, Some(bytes)))) => {
                map.insert(url, bytes);
            }
            Some(Ok((_, None))) => {}
            Some(Err(e)) => tracing::warn!(error = %e, "image prefetch task panicked"),
            None => break, // every task resolved within the budget
        }
    }
    // Budget exhausted: abort the stragglers; their images show placeholders.
    set.abort_all();
    map
}

/// Text-only message replay (no subagent reconstruction). Used as a fallback
/// for child views without persisted events, and by tests.
pub fn replay_messages(agent_name: &str, messages: &[Message]) -> ChatView {
    let empty = HashMap::new();
    let mut chat = ChatView {
        agent: sanitize_single_line(agent_name).into_owned(),
        ..Default::default()
    };
    let mut deferred_summaries: Vec<String> = Vec::new();
    for msg in messages {
        if let Some(summary) = compaction_summary_of(msg) {
            deferred_summaries.push(summary);
            continue;
        }
        replay_one(&mut chat, msg, &empty);
    }
    // Same fold as `replay_into_chat` — one shared step-shape guarantee.
    coalesce_steps(&mut chat.blocks);
    for summary in &deferred_summaries {
        push_compaction_summary_block(&mut chat, summary);
    }
    chat.context_used = estimate_messages_for_display(messages) as u64;
    chat
}
