//! Free-function helpers extracted from `app.rs` to keep that file under the
//! 800-line iteration cap. All are `pub(crate)` and re-exported by `app.rs`
//! (`pub(crate) use crate::app_helpers::*`), so existing call sites and the
//! `crate::app::*` test references keep resolving unchanged.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
pub(crate) fn sys_tokens_for(agent_name: &str, workdir: &Path, skill: Option<&str>) -> u64 {
    let agent = match resolve_agent(agent_name) {
        Some(a) => a,
        None => return 0,
    };
    let text = opencoder_session::prompt::build_system(&agent, workdir, skill).text();
    estimate(&text) as u64
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

/// Mouse-event handler extracted from `app.rs`'s main event loop. Owns all the
/// state it touches via mutable references, so it returns nothing — every effect
/// is a side effect on the caller's locals. `async` because the queue-panel
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
    queue_items: &mut Vec<(i64, String)>,
    session_id: &str,
    store: &dyn Store,
    copy_msg: &mut Option<String>,
) {
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
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let mut consumed = false;
            if let Some(r) = hits.jump_btn {
                if in_rect(r, m.column, m.row) {
                    *follow = true;
                    consumed = true;
                }
            }
            for btn in &hits.queue_btns {
                if !in_rect(btn.rect, m.column, m.row) {
                    continue;
                }
                consumed = true;
                match queue_panel::plan(queue_items, btn.seq, btn.action) {
                    queue_panel::QueueEffect::Delete(seq) => {
                        if store.delete_input(seq).await.is_ok() {
                            queue_items.retain(|(s, _)| *s != seq);
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
            // Nothing clicked: begin a text-selection drag
            // inside the body. Stored as an absolute content
            // row so it stays anchored while scrolling.
            if !consumed {
                if let Some(r) = hits.body {
                    if let Some(abs) = crate::selection::abs_row_at(r, m.row, *scroll) {
                        *selection = Some((abs, abs));
                    }
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
                if let Some(report) = crate::selection::finish_copy(viewed, hits.body, sel) {
                    *copy_msg = Some(report.status_message());
                }
                *selection = None;
            }
        }
        MouseEventKind::ScrollUp => {
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    *scroll = scroll.saturating_sub(3);
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
        let store = StubStore;
        let mut copy_msg: Option<String> = None;

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
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
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
        let store = StubStore;
        let mut copy_msg: Option<String> = None;

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
            &mut queue_items,
            "s",
            &store,
            &mut copy_msg,
        )
        .await;

        assert!(
            follow,
            "short parent legitimately pins to bottom immediately"
        );
    }
}
