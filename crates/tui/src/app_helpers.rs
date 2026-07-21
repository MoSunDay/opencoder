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

/// Decide what text to insert into the composer for a bracketed-paste event.
///
/// Decide what text to insert into the composer for a bracketed-paste event.
///
/// Dragging a file into the terminal delivers its path atomically — sometimes
/// with a trailing newline, surrounding quotes, a `file://` URI prefix, or
/// backslash-escaped spaces (terminals that quote paths containing spaces).
/// When the payload resolves to an existing file — absolute, or relative to
/// `workdir` (so a drag-pasted bare filename like `src/main.rs` also works) —
/// we echo its canonical absolute path; otherwise the raw text is returned
/// unchanged so ordinary text pastes keep working. Only payloads that point at
/// a real file on disk are rewritten, so a pasted word that is not a file is
/// never surprising.
pub(crate) fn paste_payload(payload: &str, workdir: &Path) -> String {
    // Drop a single trailing newline that many terminals append to pastes.
    let trimmed = payload
        .strip_suffix('\n')
        .or_else(|| payload.strip_suffix('\r'))
        .unwrap_or(payload);

    // Only single-line, non-empty payloads can be a file path.
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return payload.to_string();
    }

    // Strip surrounding single/double quotes and a possible `file://` scheme.
    let mut candidate = trimmed.trim_matches(|c| c == '\'' || c == '"');
    if let Some(rest) = candidate.strip_prefix("file://") {
        candidate = rest;
    }

    if let Some(full) = resolve_existing_path(candidate, workdir) {
        full.to_string_lossy().into_owned()
    } else {
        payload.to_string()
    }
}

/// If `candidate` points at an existing file, return its canonical absolute
/// form. Absolute paths are resolved directly; relative paths are resolved
/// against `workdir` (so a drag-pasted relative filename resolves to its full
/// path). Falls back to un-escaping backslash-escaped spaces that some
/// terminals insert when pasting paths containing spaces.
fn resolve_existing_path(candidate: &str, workdir: &Path) -> Option<PathBuf> {
    use std::borrow::Cow;
    let path = Path::new(candidate);
    let base: Cow<Path> = if path.is_absolute() {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(workdir.join(candidate))
    };
    if let Ok(full) = base.canonicalize() {
        return Some(full);
    }
    // Some terminals escape spaces as "\ "; retry with them un-escaped.
    let unescaped: String = candidate.replace("\\ ", " ");
    if unescaped != candidate {
        let base2: std::path::PathBuf = if Path::new(&unescaped).is_absolute() {
            Path::new(&unescaped).to_path_buf()
        } else {
            workdir.join(&unescaped)
        };
        if let Ok(full) = base2.canonicalize() {
            return Some(full);
        }
    }
    None
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

/// Drop every pending steer/queue input from the store and reset both
/// in-memory mirrors. Used on double-Esc hard-abort (`KeyAction::Cancel`)
/// so buffered inputs don't resurface on resume. `delete_input` only
/// touches rows whose `promoted_seq IS NULL`, so fanning out over both
/// mirrors is safe even if the runner already promoted/consumed some.
pub(crate) async fn clear_pending_inputs(
    store: &dyn Store,
    steer_items: &mut Vec<(i64, String)>,
    queue_items: &mut Vec<(i64, String)>,
) {
    for (seq, _) in steer_items.iter().chain(queue_items.iter()) {
        let _ = store.delete_input(*seq).await;
    }
    steer_items.clear();
    queue_items.clear();
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
/// The ambient global `~/.opencoder/AGENTS.md` is excluded from this count so
/// the context meter at startup (and throughout the session) is not inflated
/// by an always-on global instructions file. The global content still ships
/// in the system prompt; only the accounting omits it.
pub(crate) fn sys_tokens_for(agent_name: &str, workdir: &Path, skill: Option<&str>) -> u64 {
    let agent = match resolve_agent(agent_name) {
        Some(a) => a,
        None => return 0,
    };
    let text = opencoder_session::prompt::build_system(
        &agent,
        workdir,
        skill,
        &opencoder_core::CapabilitiesConfig::default(),
    )
    .text();
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
                            chat.steer_items.retain(|(s, _)| *s != seq);
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
#[path = "app_helpers_tests.rs"]
mod tests;
