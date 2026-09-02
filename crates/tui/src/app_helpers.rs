//! Free-function helpers extracted from `app.rs` to keep that file under the
//! 800-line iteration cap. All are `pub(crate)` and re-exported by `app.rs`
//! (`pub(crate) use crate::app_helpers::*`), so existing call sites and the
//! `crate::app::*` test references keep resolving unchanged.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use opencoder_core::{resolve_agent, Config, Endpoint};
use opencoder_llm::estimate;
use opencoder_session::SessionState;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionPatch, Store};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::chat::ChatView;
use crate::keymap::KeyBindings;
use crate::theme;
use crate::worker::UiCmd;

use crate::queue_panel;
use crate::render::{in_rect, MouseHits};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

#[cfg(test)]
#[cfg(test)]
pub(crate) use crate::resize::size_changed;
pub(crate) use crate::resize::{on_resize_event, poll_idle_resize};

/// Copy-paste-ready command to resume a session by id.
pub(crate) fn resume_hint(id: &str) -> String {
    format!("resume with: opencoder -s {id}")
}

/// Re-apply an explicit `--model` to a resumed/newly-built session. `resume()`
/// restores the model stored in the session row into `session.config.model`,
/// so an explicit `--model` must win here. Returns the new model string when
/// the session changed (caller persists it), else `None`. Mirrors the headless
/// path in `crates/cli/src/run.rs` -- the TUI previously lacked this and
/// silently dropped `--model` on resume (chosen model not applied after restart).
pub(crate) fn reapply_session_model(
    session: &mut SessionState,
    model: &Option<String>,
) -> Option<String> {
    let m = model.as_ref()?;
    if session.config.model == *m {
        return None;
    }
    session.config.model = m.clone();
    session.model = session.config.model_id().to_string();
    Some(m.clone())
}

/// Persist a model change (the `Some` returned by [`reapply_session_model`])
/// back into the session row so subsequent resumes honor the new choice.
pub(crate) async fn persist_session_model(store: &dyn Store, id: &str, model: String) {
    let _ = store
        .update_session(
            id,
            &SessionPatch {
                model: Some(model),
                updated_at: Some(opencoder_core::message::now_ms()),
                ..Default::default()
            },
        )
        .await;
}

/// Resolve the `(base_url, api_key)` pair used to build the LLM client at TUI
/// startup. Selects the provider whose name matches the `model`'s `provider/`
/// prefix via `Config::resolve_endpoint`, so a `model` like
/// `deepseek/deepseek-chat` resolves against `providers["deepseek"]` rather
/// than the legacy top-level `provider.base_url`. Extracted as a testable seam
/// for the startup path, which otherwise only runs inside `run`.
pub(crate) fn startup_endpoint(config: &Config) -> Result<Endpoint> {
    Ok(config.resolve_endpoint()?)
}

/// Build the initial `ChatView` for `run_app`: replay persisted history for a
/// resumed session so the transcript is visible on startup, else a blank view.
pub(crate) async fn initial_chat_view(
    session: &SessionState,
    store: &Arc<dyn Store>,
) -> crate::chat::ChatView {
    let view = if !session.messages.is_empty() {
        crate::session_ui::replay_into_chat(
            &session.agent.name,
            &session.messages,
            store,
            &session.id,
            // Cold start: no live view whose accumulated token cost to floor.
            0,
        )
        .await
    } else {
        crate::chat::ChatView {
            agent: crate::terminal_text::sanitize_single_line(&session.agent.name).into_owned(),
            ..Default::default()
        }
    };
    view
}

/// Pre-`handle_key` intercepts that run while no modal is open: Esc or Ctrl+L
/// exits a subagent view back to the parent at FOLLOW MODE (bottom of view);
/// Ctrl+L additionally collapses all thinking + tool-output blocks and clears
/// the input; Ctrl+F forces a full-screen redraw.
/// Returns `true` when the key was consumed (caller should `continue` to the
/// next event).
///
/// Mode switching is deliberately not intercepted here: Ctrl+T falls through
/// to `handle_key`, which returns the explicit parent-agent switch action.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pre_key_intercept(
    k: KeyEvent,
    bindings: &KeyBindings,
    subagent_focus: &mut Option<usize>,
    follow: &mut bool,
    last_esc: &mut Option<Instant>,
    chat: &mut ChatView,
    input: &mut String,
    cursor_idx: &mut usize,
    needs_clear: &mut bool,
    sidecar_ask: &mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
) -> bool {
    *needs_clear = false;
    // Sidecar ctx-switch: Esc DESTROYS the sidecar — the actor drops its
    // conversation (aborting an in-flight turn; partial usage still lands on
    // the main session) and every sidecar block is purged from the
    // transcript, so the parent view carries zero sidecar trace.
    if chat.sidecar_focus && k.code == KeyCode::Esc {
        crate::sidecar_ui::exit_panel(chat, sidecar_ask);
        *follow = true; // follow mode: render clamps scroll to bottom (render.rs)
        *last_esc = None;
        return true;
    }
    // Subagent ctx-switch: Esc exits to parent view.
    if subagent_focus.is_some() && k.code == KeyCode::Esc {
        *subagent_focus = None;
        *follow = true; // follow mode: render clamps scroll to bottom (render.rs)
        *last_esc = None;
        return true;
    }
    // collapse_blocks (default: Ctrl+L): collapse all thinking + tool-output
    // blocks, exit subagent view if in one, return to follow mode, clear input.
    if bindings.collapse_blocks.matches(&k) {
        if let Some(idx) = *subagent_focus {
            if let Some(crate::chat::ChatBlock::Subagent { view, .. }) = chat.blocks.get_mut(idx) {
                view.collapse_all_collapsible();
            }
            *subagent_focus = None;
            *last_esc = None;
        }
        // Same exit path for a focused sidecar box: DESTROY it (the sidecar
        // is a temporary bypass, not a transcript artifact), then fall
        // through to the parent-wide collapse below.
        if chat.sidecar_focus {
            crate::sidecar_ui::exit_panel(chat, sidecar_ask);
            *last_esc = None;
        }
        chat.collapse_all_collapsible();
        *follow = true; // follow mode: render clamps scroll to bottom (render.rs)
        input.clear();
        *cursor_idx = 0;
        return true;
    }
    // force_redraw (default: Ctrl+F): force a full-screen redraw. The caller
    // resets the terminal's diff buffer via `terminal.clear()`.
    if bindings.force_redraw.matches(&k) {
        *needs_clear = true;
        return true;
    }
    false
}

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

pub(crate) fn mk_input_with_images(
    session_id: &str,
    delivery: Delivery,
    prompt: &str,
    display_text: Option<String>,
    images: &[String],
) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: session_id.to_string(),
        delivery,
        prompt: prompt.to_string(),
        images: images.to_vec(),
        display_text,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Drain pending clipboard/paste images into a plain URI vector, clearing the
/// pending buffer in one step. Every submit path (text, pure-skill, steer,
/// queue) uses this so an attached image is never silently dropped nor leaked
/// onto a later, unrelated submission.
#[cfg(test)]
pub(crate) fn drain_pending_images(pending: &mut Vec<(String, String)>) -> Vec<String> {
    let uris: Vec<String> = pending.iter().map(|(u, _)| u.clone()).collect();
    pending.clear();
    uris
}

/// Snapshot image URIs from the pending buffer **without** clearing it. Pair
/// with `pending_images.clear()` on the success path so images are only
/// consumed when the store write or worker dispatch actually succeeds —
/// avoiding silent data loss on store errors or dead workers.
pub(crate) fn snapshot_image_uris(pending: &[(String, String)]) -> Vec<String> {
    pending.iter().map(|(u, _)| u.clone()).collect()
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
        Style::default().fg(theme::err_color()),
    )));
}

/// Estimated tokens of the system prompt that will accompany every request:
/// `agent.prompt + project instructions + environment block` plus the
/// latent-tool schemas an active skill unlocks and the one-line active-skill
/// tail reminder. The skill body itself no longer ships in the system text
/// (it moved to a transient tail message); the catalog-reminder tokens
/// (config-dependent, tiny) are not counted here.
/// Tracked separately from `ChatView::context_used` (which sums the streamed
/// transcript and resets on compaction) so the context meter reflects the
/// real request size — including the global `~/.opencoder/AGENTS.md` content,
/// which ships in the system prompt and consumes context like any other part.
/// Refresh the local active-skill mirrors from the shared `skill_prompt`
/// handle after the runner activated (or cleared) a skill at **consumption**
/// time (queue/steer drain resolved a `$name` token at the idle boundary).
/// The runner shares only the body, so the display name is derived from the
/// body's `> Source: .../skills/<name>/SKILL.md` prefix; `sys_tokens` is
/// re-estimated from the new body, and the `[act]` task-plan chip highlight
/// is re-derived. The early-return (body unchanged) path must keep the
/// caller's `plan_skill_active` value as-is: a yellow cleared by a steer/
/// queued input taking effect must not be revived by a later idle mirror
/// refresh. No-op while both sides agree.
pub(crate) fn refresh_skill_mirrors(
    skill_handle: &Arc<Mutex<Option<String>>>,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    plan_skill_active: &mut bool,
) {
    let body = skill_handle.lock().ok().and_then(|g| g.clone());
    if body == *active_skill_body {
        return;
    }
    *active_skill = body
        .as_deref()
        .and_then(crate::skill_display::skill_name_from_body);
    *active_skill_body = body;
    *sys_tokens = sys_tokens_for(agent_name, workdir, active_skill_body.as_deref());
    *plan_skill_active = crate::skill_persist::act_plan_highlight(active_skill.as_deref());
}

pub(crate) fn sys_tokens_for(agent_name: &str, workdir: &Path, skill: Option<&str>) -> u64 {
    let agent = match resolve_agent(agent_name) {
        Some(a) => a,
        None => return 0,
    };
    let text = opencoder_session::prompt::build_system(&agent, workdir, None, skill).text();
    let registry = opencoder_session::tools::registry();
    let tool_tokens =
        opencoder_session::tools::estimate_tool_schema_tokens(&agent, skill, &registry);
    let tail = skill
        .and_then(opencoder_session::skill_context::source_path_from_body)
        .map(|path| opencoder_session::skill_context::reminder_text(&[], Some(path)))
        .map(|t| estimate(&t))
        .unwrap_or(0);
    estimate(&text) as u64 + tool_tokens as u64 + tail as u64
}

/// Resolve inline `$name` skill tokens in `text`: strip them from the
/// returned text and, when at least one named skill resolves, activate it
/// (one-shot: armed for the run this prompt triggers, cleared at its end) by
/// updating the skill state and writing the resolved body into the
/// shared `Arc<Mutex<Option<String>>>` skill handle. Returns
/// `(clean_text, unresolved_names)` — names that appeared in tokens but matched
/// no discovered skill, so the caller can warn the user.
///
/// When no tokens are present the active skill is left untouched (still
/// armed).
/// When tokens are present but none resolve, the skill is likewise untouched
/// and every name is reported as unresolved. The shared skill handle is updated
/// directly before the caller issues `Prompt`, so the worker — which holds the
/// same `Arc` — observes the new skill on its next turn without a channel hop.
/// Core skill-token resolver: maps `$name` tokens against an *explicit*
/// skill slice instead of scanning `~/.opencoder/skills`. Taking skills as a
/// parameter removes the process-global `HOME` read entirely —
/// `std::env::set_var` is not thread-safe at the libc level, so under parallel
/// test execution a concurrent `getenv` could observe a transiently-wrong HOME
/// and spuriously mark a known skill unresolved. Production callers discover
/// skills via `opencoder_core::discover_skills()` and pass them in explicitly.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_skill_tokens_with(
    skills: &[opencoder_core::Skill],
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
    let mut resolved_names: Vec<String> = Vec::new();
    let mut resolved_bodies: Vec<String> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for n in &unique {
        if let Some(sk) = skills.iter().find(|s| &s.name == n) {
            resolved_names.push(sk.name.clone());
            resolved_bodies.push(opencoder_core::body_with_source(sk));
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
        *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(body);
    }
    // Rebuild `clean` so that ONLY resolved tokens are stripped — unresolved
    // `$name` bytes are preserved verbatim as literal text, preventing content
    // loss (e.g. a glued `$review1) task` keeps the `1)` instead of vanishing).
    let resolved_set: std::collections::HashSet<String> = resolved_names.iter().cloned().collect();
    let clean = crate::skill_token::strip_resolved_skill_tokens(text, &resolved_set);
    (clean, unresolved)
}

/// Resolves `$name` tokens against an *explicit* skill slice (typically
/// `discover_in(tempdir)`) and pushes a warning marker for unresolved skills.
/// The 9th arg (`chat`) is load-bearing: it lets the caller avoid a separate
/// `push_marker` round-trip after every submit/steer/queue. Production callers
/// discover skills via `opencoder_core::discover_skills()` and pass them in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_and_warn_with(
    skills: &[opencoder_core::Skill],
    text: &str,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    sys_tokens: &mut u64,
    agent_name: &str,
    workdir: &Path,
    skill_handle: &Arc<Mutex<Option<String>>>,
    chat: &mut ChatView,
) -> (String, Vec<String>) {
    let (clean, unresolved) = apply_skill_tokens_with(
        skills,
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
            Style::default().fg(theme::warn_color()),
        )));
    }
    (clean, unresolved)
}

/// Record a submitted/steered/queued input so Up/Down arrow can recall it,
/// WITHOUT echoing a transcript marker (the steer/queue panels already
/// display the text).
/// Transient flash for `KeyAction::QueueUnsupported`: a Tab-queue was
/// rejected because a running subagent is focused — the user should press
/// Enter to steer the subagent instead. Extracted from `app.rs` (file-size
/// cap); the parent session is deliberately left untouched by the caller.
pub(crate) fn queue_unsupported_flash(anim_tick: u32) -> (String, u32) {
    (
        "\u{26a0} tab queue not supported for subagents \u{2014} press Enter to steer".to_string(),
        anim_tick,
    )
}

/// Stable busy hint shared by direct shortcuts and textual mode commands.
pub(crate) fn mode_switch_busy_flash(anim_tick: u32) -> (String, u32) {
    (
        "\u{23f3} busy \u{2014} mode switch blocked, retry when idle".to_string(),
        anim_tick,
    )
}

pub(crate) fn push_history(history: &mut Vec<String>, hist_idx: &mut Option<usize>, text: &str) {
    history.push(text.to_string());
    *hist_idx = None;
}

/// Echo a submitted prompt: `echo` is the model-facing text rendered as the
/// transcript user block (a compound control command's tail — the command
/// token itself never echoes), `history_text` is the raw input kept for
/// arrow-up recall. `first_prompt` (title source) stays keyed on the raw
/// input so slash-prefixed inputs keep being excluded.
pub(crate) fn push_user(
    chat: &mut ChatView,
    history: &mut Vec<String>,
    hist_idx: &mut Option<usize>,
    echo: &str,
    history_text: &str,
) {
    if chat.first_prompt.is_none() {
        let t = history_text.trim();
        if !t.is_empty() && !t.starts_with('/') {
            chat.first_prompt = Some(t.to_string());
        }
    }
    push_history(history, hist_idx, history_text);
    chat.blocks.push(crate::chat::ChatBlock::User {
        rendered: crate::markdown::render(echo),
    });
    chat.push_marker(Line::from(""));
}

pub(crate) use opencoder_core::data_dir_for;

/// Outcome of a mouse event: `None` for normal handling (all effects are side
/// effects on the caller's locals), or `SteerSubmit` when the user clicked the
/// `>` submit-now button on a steer row, signalling the caller to interrupt the
/// current turn and restart the drain loop to promote pending steers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MouseOutcome {
    None,
    SteerSubmit,
}

/// Which `ChatView` a header click toggles: the focused subagent's child view
/// when one is active, else the parent. `None` (click still consumed) for a
/// stale or non-Subagent focus index.
fn collapse_view(chat: &mut ChatView, focus: Option<usize>) -> Option<&mut ChatView> {
    let i = match focus {
        None => return Some(chat),
        Some(i) => i,
    };
    match chat.blocks.get_mut(i)? {
        crate::chat::ChatBlock::Subagent { view, .. } => Some(view),
        _ => None,
    }
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
    scroll: &mut u32,
    follow: &mut bool,
    chat: &mut ChatView,
    subagent_focus: &mut Option<usize>,
    subagent_sys: &mut u64,
    workdir: &Path,
    queue_items: &mut Vec<(i64, String)>,
    session_id: &str,
    store: &dyn Store,
    queue_scroll: &mut u32,
    pending_images: &mut Vec<(String, String)>,
) -> MouseOutcome {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Follow button: highest-priority check so a quick succession of
            // body-click + arrow-click does not have the arrow-click swallowed.
            if let Some(r) = hits.jump_btn {
                if in_rect(r, m.column, m.row) {
                    *follow = true;
                    return MouseOutcome::None; // deterministic jump to bottom
                }
            }

            // Top-jump button: scroll back to the very first row. Sits next to
            // the jump_btn check.
            if let Some(r) = hits.top_btn {
                if in_rect(r, m.column, m.row) {
                    *scroll = 0;
                    *follow = false;
                    return MouseOutcome::None; // jump to top
                }
            }

            // ── Button-hit detection ──
            // Queue / Thinking / Subagent affordances must respond on the
            // FIRST click (no early-return guard before the toggle loop).
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
            // Attachment ✕ delete: remove the clicked pending image on the
            // FIRST click, like the queue buttons. Rects are rebuilt every
            // frame and a single click removes one image, so the index stays
            // valid for this event only.
            for btn in &hits.attach_del_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if btn.index < pending_images.len() {
                        pending_images.remove(btn.index);
                    }
                    consumed = true;
                    break;
                }
            }
            // Click a Thinking/Tool header to toggle its collapse (subagent-aware:
            // toggles the focused child view, not the parent).
            for btn in &hits.thinking_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::toggle_thinking_at(v, btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            for btn in &hits.tool_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::cycle_tool_group_at(v, btn.block_idx);
                    }
                    consumed = true;
                    break;
                }
            }
            for btn in &hits.compaction_btns {
                if in_rect(btn.rect, m.column, m.row) {
                    if let Some(v) = collapse_view(chat, *subagent_focus) {
                        ChatView::toggle_compaction_at(v, btn.block_idx);
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
                    *scroll = 0;
                    *follow = true;
                    *subagent_focus = Some(btn.block_idx);
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
                return MouseOutcome::None;
            }
        }
        MouseEventKind::ScrollUp => {
            // Wheel-up over the queue/steer panel looks at older entries (toward the top; rects never overlap the body).
            if let Some(r) = hits.queue_panel {
                if in_rect(r, m.column, m.row) {
                    *queue_scroll = queue_scroll.saturating_sub(1);
                    return MouseOutcome::None;
                }
            }
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    *scroll = scroll.saturating_sub(8);
                    *follow = false;
                }
            }
        }
        MouseEventKind::ScrollDown => {
            // Wheel-down over the queue/steer panel moves toward newer entries (toward the bottom).
            if let Some(r) = hits.queue_panel {
                if in_rect(r, m.column, m.row) {
                    // Clamp to the cached panel total (mirrors the body clamp) so burst wheels can't overshoot.
                    let max_scroll = hits.queue_total.saturating_sub(r.height as usize);
                    *queue_scroll = queue_scroll.saturating_add(1).min(max_scroll as u32);
                    return MouseOutcome::None;
                }
            }
            if let Some(r) = hits.body {
                if in_rect(r, m.column, m.row) {
                    let visible_h = r.height.saturating_sub(2) as usize;
                    // Use cached total_rows from the last render_body call instead
                    // of re-flattening the entire transcript on every wheel event.
                    let total_rows = hits.total_rows;
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

/// Open (creating its data dir if needed) the on-disk sqlite store rooted at
/// `workdir`. Best-effort dir creation: a mkdir failure is ignored via `.ok()`
/// so the subsequent store-open surfaces the real error. Extracted from
/// `app::run` to keep that file under the 800-line iteration cap.
pub(crate) async fn open_store(workdir: &Path) -> Result<Arc<dyn Store>> {
    let data_dir = data_dir_for(workdir);
    tokio::fs::create_dir_all(&data_dir).await.ok();
    Ok(Arc::new(
        LibsqlStore::open(data_dir.join("opencoder.db")).await?,
    ))
}

/// Force a full-screen redraw when `needs_clear` is set: clears the terminal
/// diff buffer so the next frame repaints every cell, then authorises the
/// render. Called after `pre_key_intercept` reports Ctrl+F. Extracted from
/// `app::run_app` to keep that file under the 800-line iteration cap.
pub(crate) fn apply_force_redraw<B: ratatui::backend::Backend>(
    needs_clear: bool,
    terminal: &mut Terminal<B>,
    render_pending: &mut bool,
    skip_next_render: &mut bool,
) {
    if needs_clear {
        let _ = terminal.clear();
        *render_pending = true;
        *skip_next_render = false;
    }
}

#[cfg(test)]
#[path = "app_helpers_tests/mod.rs"]
mod tests;
