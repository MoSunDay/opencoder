//! Subtask integration helpers for the main event loop.
//!
//! Extracted from `app.rs` to keep that file under the 800-line iteration
//! cap (the `app_notepad` pattern): fullscreen `/subtasks` key routing, the
//! shared append-to-parent action (Ctrl+Enter and the detail view), and the
//! per-tick subtask pollers (watcher deliveries, one-shot dispatch
//! completion, capability catalog, detail refresh/cancel).

use std::sync::Arc;

use crossterm::event::KeyEvent;
use opencoder_core::Config;
use opencoder_store::Store;
use tokio::sync::mpsc;

use crate::chat::ChatView;
use crate::queue_admitter::{self, AdmitUiState};
use crate::subtask_detail::{SubtaskOutcome, SubtaskView};
use crate::subtask_events::SubtaskUi;
use crate::task::TaskPick;
use crate::worker::UiEvent;

/// Fallback terminal height when app.rs cannot probe the terminal size.
pub(crate) const FALLBACK_TERM_H: u16 = 24;

/// Route one key while the fullscreen `/subtasks` view is open. Returns
/// `true` when the key was consumed (app.rs sets dirty + continues). `Esc`
/// closes, `Ctrl+Enter` appends the selected result back to the main task,
/// `Enter` descends into the child conversation (armed with a main-task
/// return pointer, exactly like the Ctrl+Enter-from-child flow).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn key(
    view: &mut Option<SubtaskView>,
    k: KeyEvent,
    terminal_height: u16,
    store: &Arc<dyn Store>,
    session_id: &str,
    chat: &mut ChatView,
    subtask_ui: &mut SubtaskUi,
    admit_tx: &mpsc::Sender<queue_admitter::AdmitReq>,
    admit_st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    pending_task_pick: &mut Option<TaskPick>,
    config: &Config,
) -> bool {
    let Some(detail) = view.as_mut() else {
        return false;
    };
    match crate::subtask_detail::handle_key(detail, k, terminal_height, config) {
        SubtaskOutcome::Idle => {}
        SubtaskOutcome::Close => *view = None,
        SubtaskOutcome::Append(child_id) => {
            append_and_pick(
                &child_id,
                store,
                session_id,
                chat,
                subtask_ui,
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                mode_flash,
                anim_tick,
                pending_task_pick,
            )
            .await;
            *view = None;
        }
        SubtaskOutcome::EnterConversation(child_id) => {
            // Stage the parent pointer: the switch Ok arm clears the live
            // descent pointer first, then adopts this once the child is up.
            subtask_ui.arm_after_switch = Some(session_id.to_string());
            *pending_task_pick = Some(TaskPick::Resume(child_id));
            *view = None;
        }
    }
    true
}

/// Append a finished subtask's result to the parent task (shared by
/// `Ctrl+Enter` in a child session and the `/subtasks` detail view) and
/// stage the switch-back pick when the parent is not the live session.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn append_and_pick(
    child_id: &str,
    store: &Arc<dyn Store>,
    session_id: &str,
    chat: &mut ChatView,
    subtask_ui: &mut SubtaskUi,
    admit_tx: &mpsc::Sender<queue_admitter::AdmitReq>,
    admit_st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    pending_task_pick: &mut Option<TaskPick>,
) {
    if let Some(pick) = super::app_loop::append_subtask_result(
        child_id,
        store,
        session_id,
        chat,
        subtask_ui.main_task_id.as_deref(),
        admit_tx,
        admit_st,
        queue_items,
        pending_images,
        mode_flash,
        anim_tick,
    )
    .await
    {
        subtask_ui.main_task_id = None;
        *pending_task_pick = Some(pick);
    }
}

/// Per-tick subtask pollers: watcher deliveries (folded into the registry,
/// the session counters and the open detail view), one-shot dispatch
/// completion, the capability catalog, and the open view's refresh/cancel
/// requests. Returns `(dirty, render_pending)`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn tick(
    subtask_ui: &mut SubtaskUi,
    subtask_view: &mut Option<SubtaskView>,
    store: &Arc<dyn Store>,
    session_states: &mut std::collections::HashMap<String, crate::session_ui::SessionUiState>,
    session_id: &str,
    chat: &mut ChatView,
    config: &Config,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
) -> (bool, bool) {
    let mut dirty = false;
    let mut render = false;
    // Keep the `@capability` mention catalog warm (errors silent: the @
    // path flashes its own failures).
    if let Some(Ok(caps)) = subtask_ui.poll_caps() {
        subtask_ui.caps = caps;
    }
    // Subtask watcher deliveries (dedicated channel; the session worker
    // channel is quit on every switch).
    for ev in subtask_ui.drain_updates() {
        if let UiEvent::SubtaskUpdate {
            ref session,
            ref id,
            ref status,
            ref summary,
            tokens,
        } = ev
        {
            if let Some(view) = subtask_view.as_mut() {
                view.apply_update(id, status, summary.as_deref(), tokens);
                dirty = true;
            }
            if let Some((done, total, _)) = crate::subtask_events::apply_update(
                store, session_states, session_id, chat, session, id, status, summary.as_deref(),
                tokens,
            )
            .await
            {
                let _ = (done, total);
                if crate::subtask_registry::is_terminal(status) {
                    *mode_flash = Some((format!("subtask {id} {status}"), anim_tick));
                }
                dirty = true;
                render = true;
            }
        }
    }
    // One-shot `@capability` dispatch completion: flash, recompute the
    // counters, spawn the watcher and keep the catalog warm.
    if let Some(result) = subtask_ui.poll_dispatch() {
        match result {
            Ok(outcome) => {
                *mode_flash = Some((
                    format!("subtask {} 已提交", outcome.execution_id),
                    anim_tick,
                ));
                if let Ok(registry) = crate::remote::subtasks::load_registry(store, session_id).await
                {
                    chat.subtask_done = crate::subtask_registry::done_count(&registry);
                    chat.subtask_total = crate::subtask_registry::total_count(&registry);
                }
                if !chat.remote {
                    subtask_ui
                        .maybe_spawn_watcher(config, store, session_id)
                        .await;
                }
                subtask_ui.refresh_caps(config);
                dirty = true;
                render = true;
            }
            Err(error) => *mode_flash = Some((format!("{error:#}"), anim_tick)),
        }
    }
    // Open detail view: snapshot refresh + cancel completions (the poll
    // methods merge results into the view themselves).
    if let Some(view) = subtask_view.as_mut() {
        if let Some(result) = view.poll_refresh() {
            match result {
                Ok(()) => dirty = true,
                Err(error) => *mode_flash = Some((format!("{error:#}"), anim_tick)),
            }
        }
        if let Some(result) = view.poll_cancel() {
            match result {
                Ok(()) => *mode_flash = Some(("已请求取消 subtask".into(), anim_tick)),
                Err(error) => *mode_flash = Some((format!("{error:#}"), anim_tick)),
            }
        }
    }
    (dirty, render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use opencoder_core::harness::SubtaskRef;
    use opencoder_store::TursoStore;

    fn entry(id: &str) -> SubtaskRef {
        SubtaskRef {
            execution_id: id.into(),
            kind: "agent".into(),
            capability_id: "ops_x".into(),
            question: format!("question {id}"),
            status: "done".into(),
            result_summary: None,
            tokens: 0,
            created_at: 1,
        }
    }

    /// Key routing when the view is closed leaves everything untouched.
    #[tokio::test]
    async fn key_is_inert_while_the_view_is_closed() {
        let mut view: Option<SubtaskView> = None;
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        let mut subtask_ui = SubtaskUi::new();
        let (admit_tx, _rx) = mpsc::channel(4);
        let mut admit_st = AdmitUiState::default();
        let mut queue_items = Vec::new();
        let mut pending_images = Vec::new();
        let mut mode_flash = None;
        let mut pending_task_pick = None;
        let consumed = key(
            &mut view,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            FALLBACK_TERM_H,
            &store,
            "s",
            &mut chat,
            &mut subtask_ui,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            0,
            &mut pending_task_pick,
            &Config::default(),
        )
        .await;
        assert!(!consumed);
    }

    /// Esc closes the view; Enter on an agent entry arms the conversation
    /// switch (main-task pointer + Resume pick) and closes it.
    #[tokio::test]
    async fn enter_descends_into_the_selected_child_session() {
        let mut view = Some(SubtaskView::new(vec![entry("agent-a")]));
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        let mut subtask_ui = SubtaskUi::new();
        let (admit_tx, _rx) = mpsc::channel(4);
        let mut admit_st = AdmitUiState::default();
        let mut queue_items = Vec::new();
        let mut pending_images = Vec::new();
        let mut mode_flash = None;
        let mut pending_task_pick = None;
        let consumed = key(
            &mut view,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            FALLBACK_TERM_H,
            &store,
            "parent",
            &mut chat,
            &mut subtask_ui,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            0,
            &mut pending_task_pick,
            &Config::default(),
        )
        .await;
        assert!(consumed);
        assert!(view.is_none(), "fullscreen view closes on descend");
        assert_eq!(subtask_ui.arm_after_switch.as_deref(), Some("parent"));
        assert!(matches!(pending_task_pick, Some(TaskPick::Resume(id)) if id == "agent-a"));
    }
}
