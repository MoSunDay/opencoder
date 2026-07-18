//! Free-function helpers extracted from `app.rs` to keep that file under the
//! 800-line iteration cap. All are `pub(crate)` and re-exported by `app.rs`
//! (`pub(crate) use crate::app_helpers::*`), so existing call sites and the
//! `crate::app::*` test references keep resolving unchanged.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::{discover_skills, resolve_agent};
use opencoder_llm::estimate;
use opencoder_store::{Delivery, SessionInput, Store};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use crate::worker::UiCmd;

use crate::queue_panel;
use crate::render::{in_rect, MouseHits};
use crate::selection::SelRange;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::widgets::{Paragraph, Wrap};

/// Maximum interval (ms) between two left-clicks to count as a double-click.
const DBL_CLICK_MS: u64 = 400;

/// Pre-`handle_key` intercepts that run while no modal is open: Esc exits a
/// subagent view, and Ctrl+L collapses all thinking blocks / exits a
/// subagent view / clears the input. Returns `true` when the key was
/// consumed (caller should `continue` to the next event).
#[allow(clippy::too_many_arguments)]
pub(crate) fn pre_key_intercept(
    k: KeyEvent,
    subagent_focus: &mut Option<usize>,
    scroll: &mut u16,
    follow: &mut bool,
    selection: &mut Option<SelRange>,
    last_esc: &mut Option<Instant>,
    chat: &mut ChatView,
    input: &mut String,
    cursor_idx: &mut usize,
    parent_scroll: u16,
    parent_follow: bool,
) -> bool {
    // Subagent ctx-switch: Esc exits to parent view.
    if subagent_focus.is_some() && k.code == KeyCode::Esc {
        *subagent_focus = None;
        *scroll = parent_scroll;
        *follow = parent_follow;
        *selection = None;
        *last_esc = None;
        return true;
    }
    // Ctrl+L: collapse all thinking blocks, exit subagent view if in one,
    // and clear the input box.
    if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('l')) {
        if let Some(idx) = *subagent_focus {
            if let Some(crate::chat::ChatBlock::Subagent { view, .. }) = chat.blocks.get_mut(idx) {
                view.collapse_all_thinking();
            }
            *subagent_focus = None;
            *scroll = parent_scroll;
            *follow = parent_follow;
            *selection = None;
            *last_esc = None;
        }
        chat.collapse_all_thinking();
        input.clear();
        *cursor_idx = 0;
        return true;
    }
    false
}

pub(crate) fn mk_input(session_id: &str, delivery: Delivery, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.to_string(),
        delivery,
        prompt: prompt.to_string(),
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Begin a new worker turn with a fresh, uncancelled cancellation token.
///
/// The loop's `cancel` handle and the worker's `sess.cancel` must point at the
/// same token so double-Esc still targets the live turn. Refreshing on every
/// turn start is what unblocks submission after a prior double-Esc abort —
/// without it `sess.cancel` stays permanently cancelled and `run_loop`'s
/// top-of-loop `is_cancelled()` check rejects every subsequent prompt. FIFO
/// ordering on the single-consumer command channel guarantees the worker
/// applies `ResetCancel` before processing the work command.
///
/// Returns `false` if the command channel is closed — i.e. the worker task has
/// died (panic or unexpected exit). The caller treats this as fatal: pushes a
/// marker and breaks. Because input collection runs on its own thread, the UI
/// stays interactive (Ctrl+C/D still work) so the user exits cleanly instead
/// of facing a wedged spinner.
pub(crate) async fn start_turn(
    cmd_tx: &mpsc::Sender<UiCmd>,
    cancel: &mut CancellationToken,
    cmd: UiCmd,
) -> bool {
    let fresh = CancellationToken::new();
    *cancel = fresh.clone();
    if cmd_tx.send(UiCmd::ResetCancel(fresh)).await.is_err() {
        return false;
    }
    cmd_tx.send(cmd).await.is_ok()
}

/// Record that the worker task is gone and the session can no longer progress.
/// Called at every turn-start site when `start_turn` reports the worker dead;
/// the caller then breaks the main loop.
pub(crate) fn worker_dead(chat: &mut ChatView) {
    chat.push_marker(Line::from(Span::styled(
        "[worker stopped] session engine exited unexpectedly — please restart",
        Style::default().fg(Color::Red),
    )));
}

/// Estimated tokens of the system prompt that will accompany every request:
/// `agent.prompt + environment block + active skill`. Tracked separately from
/// `ChatView::context_used` (which sums the streamed transcript and resets on
/// compaction) so the context meter reflects the real request size.
///
/// The ambient global `~/.opencode/AGENTS.md` is excluded from this count so
/// the context meter at startup (and throughout the session) is not inflated
/// by an always-on global instructions file. The global content still ships
/// in the system prompt; only the accounting omits it.
pub(crate) fn sys_tokens_for(agent_name: &str, workdir: &Path, skill: Option<&str>) -> u64 {
    let agent = match resolve_agent(agent_name) {
        Some(a) => a,
        None => return 0,
    };
    let text = opencoder_session::prompt::build_system(&agent, workdir, skill).text();
    let mut tokens = estimate(&text) as u64;
    if let Some(global) = opencoder_session::prompt::global_instructions_text(workdir) {
        tokens = tokens.saturating_sub(estimate(&global) as u64);
    }
    tokens
}

/// Resolve inline `{$name}` skill tokens in `text`: strip them from the
/// returned text and, when at least one named skill resolves, activate it
/// (sticky) by updating the skill state and writing the resolved body into the
/// shared `Arc<Mutex<Option<String>>>` skill handle. Returns
/// `(clean_text, unresolved_names)` — names that appeared in tokens but matched
/// no discovered skill, so the caller can warn the user.
///
/// When no tokens are present the active skill is left untouched (sticky).
/// When tokens are present but none resolve, the skill is likewise untouched
/// and every name is reported as unresolved. The shared skill handle is updated
/// directly before the caller issues `Prompt`, so the worker — which holds the
/// same `Arc` — observes the new skill on its next turn without a channel hop.
pub(crate) fn apply_skill_tokens(
    text: &str,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
) -> (String, Vec<String>) {
    let (clean, names) = crate::skill_token::extract_skill_tokens(text);
    if names.is_empty() {
        return (clean, Vec::new());
    }
    // Dedupe names preserving first-seen order.
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for n in names {
        if seen.insert(n.clone()) {
            unique.push(n);
        }
    }
    let skills = discover_skills();
    let mut resolved_names: Vec<String> = Vec::new();
    let mut resolved_bodies: Vec<String> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for n in &unique {
        if let Some(sk) = skills.iter().find(|s| &s.name == n) {
            resolved_names.push(sk.name.clone());
            resolved_bodies.push(sk.body.clone());
        } else {
            unresolved.push(n.clone());
        }
    }
    if !resolved_bodies.is_empty() {
        let body = resolved_bodies.join("\n\n");
        let display = resolved_names.join(", ");
        *active_skill = Some(display);
        *active_skill_body = Some(body.clone());
        *sys_tokens = sys_tokens_for(agent_name, workdir, Some(&body));
        *skill_handle.lock().unwrap() = Some(body);
    }
    (clean, unresolved)
}

/// Wraps `apply_skill_tokens` with a `chat` sink for unresolved-skill warnings.
/// The 8th arg (`chat`) is load-bearing: it lets the caller avoid a separate
/// `push_marker` round-trip after every submit/steer/queue.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_and_warn(
    text: &str,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    chat: &mut ChatView,
) -> (String, Vec<String>) {
    let (clean, unresolved) = apply_skill_tokens(
        text,
        active_skill,
        active_skill_body,
        sys_tokens,
        agent_name,
        workdir,
        skill_handle,
    );
    if !unresolved.is_empty() {
        chat.push_marker(Line::from(Span::styled(
            format!("\u{26a0} unknown skill: {}", unresolved.join(", ")),
            Style::default().fg(Color::Yellow),
        )));
    }
    (clean, unresolved)
}

pub(crate) fn push_user(
    chat: &mut ChatView,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
    text: &str,
) {
    history.push(text.to_string());
    *hist_idx = None;
    chat.push_marker(Line::from(Span::styled(
        format!("user: {text}"),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    chat.push_marker(Line::from(""));
}

pub(crate) fn data_dir_for(workdir: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workdir.hash(&mut h);
    let digest = h.finish();
    let mut base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("opencoder");
    base.push(format!("{digest:x}"));
    base
}

/// Outcome of a mouse event: `None` for normal handling (all effects are side
/// effects on the caller's locals), or `SteerSubmit` when the user clicked the
/// `>` submit-now button on a steer row, signalling the caller to interrupt the
/// current turn and restart the drain loop to promote pending steers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MouseOutcome {
    None,
    SteerSubmit,
}

/// Mouse-event handler extracted from `app.rs`'s main event loop. Owns all the
/// state it touches via mutable references, so most effects are side effects on
/// the caller's locals; the exception is `SteerSubmit` which the caller must
/// handle by restarting the drain loop. `async` because the queue-panel
/// delete/swap paths call through the `Store` trait (`delete_input` /
/// `swap_input_order`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_mouse(
    m: MouseEvent,
    hits: &MouseHits,
    scroll: &mut u16,
    follow: &mut bool,
    selection: &mut Option<SelRange>,
    chat: &mut ChatView,
    subagent_focus: &mut Option<usize>,
    parent_scroll: &mut u16,
    parent_follow: &mut bool,
    subagent_sys: &mut u64,
    workdir: &Path,
    steer_items: &mut Vec<(i64, String)>,
    queue_items: &mut Vec<(i64, String)>,
    session_id: &str,
    store: &dyn Store,
    copy_msg: &mut Option<String>,
    last_click: &mut Option<Instant>,
    dbl_click: &mut bool,
) -> MouseOutcome {
    // Shift+drag bypass: when Shift is held during a left-button Down or Drag,
    // return immediately so the terminal can perform its own native selection
    // (which works even when OSC52 is blocked by tmux/screen or the terminal).
    // Also clear any in-progress app-layer selection so the overlay doesn't
    // linger. Up events are NOT bypassed so a non-Shift drag that started
    // normally still completes its copy.
    if m.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(
            m.kind,
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
        )
    {
        *selection = None;
        return MouseOutcome::None;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Follow button: highest-priority check — MUST precede double-click
            // detection so a quick succession of body-click + arrow-click does
            // not have the arrow-click swallowed by the 400 ms dbl-click guard.
            if let Some(r) = hits.jump_btn {
                if in_rect(r, m.column, m.row) {
                    *follow = true;
                    *selection = None;
                    *dbl_click = false;
                    *last_click = Some(Instant::now());
                    return MouseOutcome::None; // deterministic jump to bottom
                }
            }

            // Top-jump button: scroll back to the very first row. Sits next to
            // the jump_btn check and likewise precedes dbl-click detection.
            if let Some(r) = hits.top_btn {
                if in_rect(r, m.column, m.row) {
                    *scroll = 0;
                    *follow = false;
                    *selection = None;
                    *dbl_click = false;
                    *last_click = Some(Instant::now());
                    return MouseOutcome::None; // jump to top
                }
            }

            // ── Button-hit detection (BEFORE the dbl-click guard) ──
            // Queue / Thinking / Subagent affordances must respond on the
            // FIRST click. The 400 ms double-click window further down is meant
            // ONLY for selecting a line of body text, so it must NOT swallow a
            // header/button click that lands within 400 ms of a previous click.
            // That was the bug that made Thinking expansion probabilistic: the
            // second of two quick clicks — or any click soon after a body click
            // — hit the dbl-click early-return and never reached the toggle
            // loop. jump_btn/top_btn already precede the guard for the same
            // reason; queue/thinking/subagent now do too.
            let now = Instant::now();
            let mut consumed = false;
            for btn in &hits.queue_btns {
                if !in_rect(btn.rect, m.column, m.row) {
                    continue;
                }
                consumed = true;
                // Submit-now on a steer row: signal the caller to interrupt
                // and restart the drain loop. No store mutation needed — the
                // steers are promoted by `claim_steers()` at the top of the
                // next `run_loop` iteration.
                if btn.action == queue_panel::QueueBtnAction::Submit {
                    return MouseOutcome::SteerSubmit;
                }
                match queue_panel::plan(queue_items, btn.seq, btn.action) {
                    queue_panel::QueueEffect::Delete(seq) => {
                        if store.delete_input(seq).await.is_ok() {
                            queue_items.retain(|(s, _)| *s != seq);
                            steer_items.retain(|(s, _)| *s != seq);
                        }
                    }
                    queue_panel::QueueEffect::Swap(a, b) => {
                        if store.swap_input_order(session_id, a, b).await.is_ok() {
                            queue_panel::apply_swap(queue_items, a, b);
                        }
                    }
                    queue_panel::QueueEffect::None => {}
                }
                break;
            }
            // Click on a Thinking-block header toggles its
            // collapse state (default collapsed → expand).
            // When viewing a subagent's perspective, toggle
            // the CHILD view (the hit-rects are computed
            // from the displayed child ChatView, so the
            // block_idx refers to its blocks, not the
            // parent's — toggling the parent here was the
            // bug that made thinking unopenable in a
            // subagent view).
            for btn in &hits.thinking_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(idx) = *subagent_focus {
                        if let Some(crate::chat::ChatBlock::Subagent { view, .. }) =
                            chat.blocks.get_mut(idx)
                        {
                            view.toggle_thinking_at(btn.block_idx);
                        }
                    } else {
                        chat.toggle_thinking_at(btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            // Click on a Subagent-block header: enter
            // the subagent's perspective (ctx-switch).
            // No inline expansion — the child view and
            // its context stats are shown full-body.
            for btn in &hits.subagent_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    *parent_scroll = *scroll;
                    *parent_follow = *follow;
                    *scroll = 0;
                    *follow = true;
                    *subagent_focus = Some(btn.block_idx);
                    *selection = None;
                    // Cache subagent's system-prompt
                    // token estimate once on entry.
                    if let Some(crate::chat::ChatBlock::Subagent { kind, .. }) =
                        chat.blocks.get(btn.block_idx)
                    {
                        *subagent_sys = sys_tokens_for(kind, workdir, None);
                    }
                    consumed = true;
                    break;
                }
            }
            if consumed {
                // A button/header consumed this click: finalize exactly like
                // jump_btn does so the next click's dbl-click window starts
                // fresh from here (a toggle click must not count as the first
                // half of a body-text double-click).
                *last_click = Some(now);
                *dbl_click = false;
                return MouseOutcome::None;
            }

            // ── Double-click detection (body text only) ──
            // If this click follows a previous one within DBL_CLICK_MS, treat
            // it as the second half of a double-click: select the current line
            // and flag the selection so finish_copy copies it even though lo==hi.
            let is_dbl = last_click
                .map(|t| now.duration_since(t) < Duration::from_millis(DBL_CLICK_MS))
                .unwrap_or(false);
            *last_click = Some(now);

            if is_dbl {
                *dbl_click = true;
                if let Some(r) = hits.body {
                    if let Some(abs) = crate::selection::abs_row_at(r, m.row, *scroll) {
                        *selection = Some((abs, abs));
                    }
                }
                return MouseOutcome::None; // go straight to selection mode
            }
            *dbl_click = false;

            // No button hit and not a double-click: begin a text-selection
            // drag inside the body. Stored as an absolute content row so it
            // stays anchored while scrolling.
            if let Some(r) = hits.body {
                if let Some(abs) = crate::selection::abs_row_at(r, m.row, *scroll) {
                    *selection = Some((abs, abs));
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let (Some((anchor, _)), Some(r)) = (*selection, hits.body) {
                if let Some(abs) = crate::selection::abs_row_at(r, m.row, *scroll) {
                    *selection = Some((anchor, abs));
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(sel) = *selection {
                let viewed: &ChatView = match (*subagent_focus).and_then(|idx| chat.blocks.get(idx))
                {
                    Some(crate::chat::ChatBlock::Subagent { view, .. }) => view,
                    _ => &*chat,
                };
                if let Some(report) =
                    crate::selection::finish_copy(viewed, hits.body, sel, *dbl_click)
                {
                    *copy_msg = Some(report.status_message());
                }
                *selection = None;
            }
            *dbl_click = false;
        }
        MouseEventKind::ScrollUp => {
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    *scroll = scroll.saturating_sub(8);
                    *follow = false;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    let visible_h = r.height.saturating_sub(2) as usize;
                    let inner_w = r.width.saturating_sub(3);
                    // When a subagent perspective is focused, the scrollbar
                    // and max-rows must reflect the CHILD view's content, not
                    // the parent's. Using `chat.flatten()` (the parent) here
                    // made `follow` trip early/late whenever parent and child
                    // had different lengths — breaking wheel-scroll inside a
                    // subagent view. This mirrors the resolution the MouseUp
                    // copy path already performs below.
                    let viewed: &ChatView = match (*subagent_focus)
                        .and_then(|idx| chat.blocks.get(idx))
                    {
                        Some(crate::chat::ChatBlock::Subagent { view, .. }) => view,
                        _ => &*chat,
                    };
                    let total_rows = Paragraph::new(viewed.flatten())
                        .wrap(Wrap { trim: false })
                        .line_count(inner_w);
                    let max_rows = total_rows.saturating_sub(visible_h);
                    *scroll = scroll.saturating_add(3);
                    if (*scroll as usize) >= max_rows {
                        *follow = true;
                    }
                }
            }
        }
        _ => {}
    }
    MouseOutcome::None
}

#[cfg(test)]
mod tests {
    //! Regression tests for the mouse wheel-scroll handler, focusing on the
    //! bug where `ScrollDown` computed `max_rows` from the PARENT chat even
    //! while a subagent perspective was focused — pinning to the bottom and
    //! making the child body un-scrollable.
    use super::*;
    use async_trait::async_trait;
    use opencoder_core::Message;
    use opencoder_session::SessionEvent;
    use opencoder_store::{
        SessionEventRecord, SessionFilter, SessionListItem, SessionMeta, SessionPatch,
        SubagentTaskRecord,
    };
    use ratatui::layout::Rect;

    /// `Store` whose every method panics. The `ScrollDown` branch of
    /// `handle_mouse` never touches the store, so passing a reference is safe;
    /// if a method were ever invoked it would fail loudly.
    struct StubStore;

    #[async_trait]
    impl Store for StubStore {
        fn backend_name(&self) -> &'static str {
            "stub"
        }
        async fn create_session(&self, _: &SessionMeta) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn get_session(&self, _: &str) -> anyhow::Result<Option<SessionMeta>> {
            unimplemented!()
        }
        async fn list_sessions(&self, _: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
            unimplemented!()
        }
        async fn update_session(&self, _: &str, _: &SessionPatch) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn delete_session(&self, _: &str) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn clear_other_sessions(&self, _: &str) -> anyhow::Result<u64> {
            unimplemented!()
        }
        async fn append_message(&self, _: &str, _: &Message) -> anyhow::Result<i64> {
            unimplemented!()
        }
        async fn append_messages(&self, _: &str, _: &[Message]) -> anyhow::Result<Vec<i64>> {
            unimplemented!()
        }
        async fn load_messages(&self, _: &str) -> anyhow::Result<Vec<Message>> {
            unimplemented!()
        }
        async fn last_message_seq(&self, _: &str) -> anyhow::Result<i64> {
            unimplemented!()
        }
        async fn admit_input(&self, _: &SessionInput) -> anyhow::Result<i64> {
            unimplemented!()
        }
        async fn pending_inputs(&self, _: &str, _: Delivery) -> anyhow::Result<Vec<SessionInput>> {
            unimplemented!()
        }
        async fn promote_inputs(
            &self,
            _: &str,
            _: i64,
            _: Delivery,
        ) -> anyhow::Result<Vec<i64>> {
            unimplemented!()
        }
        async fn promote_next_queued(&self, _: &str) -> anyhow::Result<Option<i64>> {
            unimplemented!()
        }
        async fn claim_next_queue(
            &self,
            _: &str,
        ) -> anyhow::Result<Option<(i64, SessionInput)>> {
            unimplemented!()
        }
        async fn delete_input(&self, _: i64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn append_event(&self, _: &SessionEventRecord) -> anyhow::Result<i64> {
            unimplemented!()
        }
        async fn events_after(&self, _: &str, _: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
            unimplemented!()
        }
        async fn create_subagent_task(&self, _: &SubagentTaskRecord) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
            unimplemented!()
        }
        async fn list_subagent_tasks(
            &self,
            _: &str,
        ) -> anyhow::Result<Vec<SubagentTaskRecord>> {
            unimplemented!()
        }
        async fn get_subagent_task(
            &self,
            _: &str,
        ) -> anyhow::Result<Option<SubagentTaskRecord>> {
            unimplemented!()
        }
    }

    /// Parent whose own content is short but wraps a subagent whose CHILD view
    /// is long. Left unfinalized so `flatten` emits the raw lines verbatim
    /// (row count independent of the markdown renderer).
    fn parent_with_long_subagent() -> ChatView {
        let mut chat = ChatView::default();
        chat.apply(&SessionEvent::TextDelta("parent preamble".into()));
        chat.apply(&SessionEvent::Done);
        chat.apply(&SessionEvent::SubagentStart {
            id: "s1".into(),
            kind: "explore".into(),
            prompt: "find it".into(),
            child_session_id: "c1".into(),
        });
        let child_text = (0..40)
            .map(|i| format!("child output line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        chat.apply(&SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta(child_text)),
        });
        chat
    }

    fn empty_hits(body: Rect) -> MouseHits {
        MouseHits {
            jump_btn: None,
            top_btn: None,
            body: Some(body),
            queue_btns: Vec::new(),
            thinking_btns: Vec::new(),
            subagent_btns: Vec::new(),
        }
    }

    fn scroll_down() -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 40,
            row: 6,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// The regression: with a subagent focused, one wheel-down must NOT pin to
    /// the bottom even though the PARENT fits in the viewport (which, under the
    /// old parent-based `max_rows`, saturated to 0 and tripped `follow`).
    #[tokio::test]
    async fn scrolldown_in_subagent_view_uses_child_content() {
        let mut chat = parent_with_long_subagent();
        let sub_idx = chat
            .blocks
            .iter()
            .position(|b| matches!(b, crate::chat::ChatBlock::Subagent { .. }))
            .expect("a Subagent block exists");

        let parent_rows = chat.flatten().len();
        let child_rows = match &chat.blocks[sub_idx] {
            crate::chat::ChatBlock::Subagent { view, .. } => view.flatten().len(),
            _ => unreachable!(),
        };
        let body = Rect::new(0, 0, 80, 12); // visible_h = 10, inner_w = 77
        let visible_h = body.height as usize - 2;
        assert!(
            child_rows > parent_rows && child_rows > visible_h,
            "precondition: child ({child_rows}) longer than parent ({parent_rows}) and viewport ({visible_h})"
        );
        // Parent must fit in the viewport — that is what made the old math trip.
        assert!(
            parent_rows < visible_h,
            "precondition: parent ({parent_rows}) fits viewport ({visible_h})"
        );

        let hits = empty_hits(body);
        let mut scroll = 0u16;
        let mut follow = false;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus = Some(sub_idx);
        let mut parent_scroll = 0u16;
        let mut parent_follow = false;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = Vec::new();
        let mut steer_items: Vec<(i64, String)> = Vec::new();
        let store = StubStore;
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;

        handle_mouse(
            scroll_down(),
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;

        assert_eq!(scroll, 3, "scroll advanced by one notch");
        assert!(
            !follow,
            "follow must NOT trip: the child still has content below the fold"
        );
    }

    /// Mirror case: with NO subagent focused, the parent view drives `max_rows`.
    /// Here the short parent fits the viewport, so the first wheel-down
    /// legitimately pins to the bottom.
    #[tokio::test]
    async fn scrolldown_uses_parent_when_no_subagent_focused() {
        let mut chat = parent_with_long_subagent();
        let body = Rect::new(0, 0, 80, 12);
        let visible_h = body.height as usize - 2;
        assert!(
            chat.flatten().len() < visible_h,
            "precondition: parent fits viewport"
        );

        let hits = empty_hits(body);
        let mut scroll = 0u16;
        let mut follow = false;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = false;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = Vec::new();
        let mut steer_items: Vec<(i64, String)> = Vec::new();
        let store = StubStore;
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;

        handle_mouse(
            scroll_down(),
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;

        assert!(
            follow,
            "short parent legitimately pins to bottom immediately"
        );
    }

    #[tokio::test]
    async fn dbl_click_selects_line_and_copies_on_release() {
        // Build a chat view with 5 marker lines (abs rows 0-4).
        let mut chat = ChatView::default();
        for &l in &["line one", "line two", "line three", "line four", "line five"] {
            chat.push_marker(Line::from(l.to_string()));
        }

        // Body rect: inner_y=1, inner_h=10, so screen row 5 maps to abs row 4.
        let body = Rect::new(0, 0, 80, 12);
        let hits = empty_hits(body);

        let mut scroll = 0u16;
        let mut follow = true;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = true;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let mut steer_items: Vec<(i64, String)> = vec![];
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;
        let store = StubStore;

        let mk_down = |row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row,
            modifiers: KeyModifiers::NONE,
        };

        // First click — should NOT set dbl_click.
        handle_mouse(
            mk_down(5),
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(!dbl_click, "first click should not be a double-click");

        // Second click immediately — should set dbl_click and selection.
        handle_mouse(
            mk_down(5),
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(dbl_click, "second click should be detected as double-click");
        assert!(selection.is_some(), "selection should be set on dbl-click");

        // Mouse up — should copy (force=true via dbl_click).
        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            up,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(copy_msg.is_some(), "double-click should copy on release");
        assert!(selection.is_none(), "selection cleared after release");
        assert!(!dbl_click, "dbl_click reset after release");
    }

    #[tokio::test]
    async fn submit_btn_returns_steer_submit() {
        let mut chat = ChatView::default();
        let body = Rect::new(0, 0, 80, 12);

        // Build a MouseHits with a Submit button for steer seq=10 at (77, 0).
        let mut hits = empty_hits(body);
        hits.queue_btns.push(queue_panel::QueueBtn {
            seq: 10,
            action: queue_panel::QueueBtnAction::Submit,
            rect: Rect::new(77, 0, 1, 1),
        });

        let mut scroll = 0u16;
        let mut follow = true;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = true;
        let mut subagent_sys = 0u64;
        let mut steer_items: Vec<(i64, String)> = vec![(10, "redirect".into())];
        let mut queue_items: Vec<(i64, String)> = vec![];
        let store = StubStore;
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;

        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 77,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let outcome = handle_mouse(
            down,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;

        assert_eq!(
            outcome,
            MouseOutcome::SteerSubmit,
            "clicking Submit on a steer row must return SteerSubmit"
        );
        // Steer item must NOT be removed — promotion happens in the drain loop.
        assert_eq!(steer_items.len(), 1, "steer item should remain until drain");
    }

    #[tokio::test]
    async fn single_click_does_not_copy_on_release() {
        let mut chat = ChatView::default();
        for &l in &["line one", "line two", "line three", "line four", "line five"] {
            chat.push_marker(Line::from(l.to_string()));
        }

        let body = Rect::new(0, 0, 80, 12);
        let hits = empty_hits(body);

        let mut scroll = 0u16;
        let mut follow = true;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = true;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let mut steer_items: Vec<(i64, String)> = vec![];
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;
        let store = StubStore;

        let down = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            down,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(!dbl_click);

        let up = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            up,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(copy_msg.is_none(), "single click should not copy");
    }

    /// Regression: clicking the follow/jump button immediately after a body
    /// click must still work. Previously the `jump_btn` check sat AFTER the
    /// double-click guard, so the second click (within 400 ms) was swallowed
    /// by `is_dbl` and the early `return`, making the follow button
    /// unreliable.
    #[tokio::test]
    async fn jump_btn_click_works_after_recent_body_click() {
        let mut chat = ChatView::default();
        chat.push_marker(Line::from("some text"));

        let body = Rect::new(0, 0, 80, 12);
        // jump_btn sits on the body's bottom-border row, right-aligned.
        let jump_btn_rect = Rect::new(74, 11, 6, 1);
        let hits = MouseHits {
            jump_btn: Some(jump_btn_rect),
            top_btn: None,
            body: Some(body),
            queue_btns: Vec::new(),
            thinking_btns: Vec::new(),
            subagent_btns: Vec::new(),
        };

        let mut scroll = 0u16;
        let mut follow = false;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = false;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let mut steer_items: Vec<(i64, String)> = vec![];
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;
        let store = StubStore;

        // First click: hits the body interior (row 5, well inside body).
        let body_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            body_click,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(last_click.is_some(), "body click should set last_click");
        assert!(!follow, "body click should not set follow");

        // Second click immediately after (< 400 ms): hits the jump button.
        // Under the old code this was swallowed by the double-click guard.
        let jump_click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 76,
            row: 11,
            modifiers: KeyModifiers::NONE,
        };
        handle_mouse(
            jump_click,
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert!(
            follow,
            "jump button click must set follow=true even right after a body click"
        );
    }

    /// Wheel-up now advances 8 lines per notch (was 3) so scrolling back up
    /// through a long transcript feels responsive. Down is unchanged at 3.
    #[tokio::test]
    async fn scrollup_advances_faster_than_default() {
        // Build a long-enough ChatView so content clearly exceeds the small
        // viewport (visible_h = body.height - 2 = 10).
        let mut chat = ChatView::default();
        for n in 0..30u32 {
            chat.push_marker(Line::from(format!("marker line {n}")));
        }

        let body = Rect::new(0, 0, 80, 12);
        let hits = empty_hits(body);

        let scroll_up = || MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 40,
            row: 6,
            modifiers: KeyModifiers::NONE,
        };

        // `scroll` is the top-anchored line offset (0 == top); scroll-up moves
        // toward the top via `saturating_sub`. Start part-way down so a single
        // notch lands on a value that proves the 8-line step: the new step
        // yields 16 - 8 = 8, whereas the old 3-step would have left 16 - 3 = 13.
        let mut scroll = 16u16;
        let mut follow = true;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = false;
        let mut subagent_sys = 0u64;
        let mut queue_items: Vec<(i64, String)> = vec![];
        let mut steer_items: Vec<(i64, String)> = vec![];
        let mut copy_msg: Option<String> = None;
        let mut last_click: Option<Instant> = None;
        let mut dbl_click = false;
        let store = StubStore;

        handle_mouse(
            scroll_up(),
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;

        assert_eq!(scroll, 8, "one wheel-up notch now moves 8 lines (was 3)");
        assert!(!follow, "scrolling up must detach from the tail");
    }

    /// Regression: clicking a Thinking-block header must toggle on the FIRST
    /// click even when it lands within the 400 ms double-click window of a
    /// previous click. Previously the thinking-toggle loop sat AFTER the
    /// dbl-click guard, so any header click within 400 ms of a prior click was
    /// swallowed by the guard's early `return` (selecting a line instead) and
    /// the toggle never ran — making expansion probabilistic. The fix moves
    /// queue/thinking/subagent button-hit detection ahead of the guard, the
    /// same fix jump_btn/top_btn already had.
    #[tokio::test]
    async fn thinking_header_toggles_even_right_after_another_click() {
        let mut chat = ChatView::default();
        chat.apply(&SessionEvent::ReasoningDelta("secret reasoning here".into()));
        chat.apply(&SessionEvent::TextDelta("answer".into()));
        chat.apply(&SessionEvent::Done);
        // Collapsed by default: the reasoning content must NOT be visible yet.
        assert!(
            !chat
                .flatten()
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("secret reasoning"))),
            "precondition: thinking must start collapsed"
        );

        let body = Rect::new(0, 0, 80, 12);
        let header_rect = Rect::new(1, 1, 78, 1);
        let hits = MouseHits {
            jump_btn: None,
            top_btn: None,
            body: Some(body),
            queue_btns: Vec::new(),
            thinking_btns: vec![crate::render::ThinkingBtn {
                block_idx: 0,
                rect: header_rect,
            }],
            subagent_btns: Vec::new(),
        };

        let mut scroll = 0u16;
        let mut follow = false;
        let mut selection: Option<SelRange> = None;
        let mut subagent_focus: Option<usize> = None;
        let mut parent_scroll = 0u16;
        let mut parent_follow = false;
        let mut subagent_sys = 0u64;
        let mut steer_items: Vec<(i64, String)> = Vec::new();
        let mut queue_items: Vec<(i64, String)> = Vec::new();
        let store = StubStore;
        let mut copy_msg: Option<String> = None;
        // A click ~50 ms ago — squarely inside the 400 ms dbl-click window.
        // On the buggy code this trips `is_dbl` and the toggle is skipped.
        let mut last_click: Option<Instant> = Some(Instant::now());
        let mut dbl_click = false;

        let outcome = handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: header_rect.x,
                row: header_rect.y,
                modifiers: KeyModifiers::NONE,
            },
            &hits,
            &mut scroll,
            &mut follow,
            &mut selection,
            &mut chat,
            &mut subagent_focus,
            &mut parent_scroll,
            &mut parent_follow,
            &mut subagent_sys,
            Path::new("."),
            &mut steer_items,
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
            &mut last_click,
            &mut dbl_click,
        )
        .await;
        assert_eq!(outcome, MouseOutcome::None);
        assert!(
            chat.flatten()
                .iter()
                .any(|l| l.spans.iter().any(|s| s.content.contains("secret reasoning"))),
            "thinking must be expanded after the header click"
        );
        assert!(
            !dbl_click,
            "a header toggle must not be flagged as a double-click"
        );
    }
}
