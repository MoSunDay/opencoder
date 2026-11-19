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
/// `Enter` descends into the child conversation (staged together with the
/// parent return pointer, exactly like the Ctrl+Enter-from-child flow).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn key(
    view: &mut Option<SubtaskView>,
    k: KeyEvent,
    terminal_height: u16,
    store: &Arc<dyn Store>,
    session_id: &str,
    chat: &mut ChatView,
    subtask_ui: &mut SubtaskUi,
    selection: &mut crate::remote::Selection,
    admit_tx: &mpsc::Sender<queue_admitter::AdmitReq>,
    admit_st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    pending_task_pick: &mut Option<super::app_switch::PendingSwitch>,
    config: &Config,
    turn: u64,
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
                selection,
                admit_tx,
                admit_st,
                queue_items,
                pending_images,
                mode_flash,
                anim_tick,
                pending_task_pick,
                turn,
            )
            .await;
            *view = None;
        }
        SubtaskOutcome::EnterConversation(child_id) => {
            // Stage the descend atomically: drop any in-flight `/agent`
            // query, then carry the Resume pick together with the parent to
            // adopt once the child switch succeeds.
            super::app_switch::stage(
                pending_task_pick,
                selection,
                super::app_switch::descend(TaskPick::Resume(child_id), session_id.to_string()),
            );
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
    selection: &mut crate::remote::Selection,
    admit_tx: &mpsc::Sender<queue_admitter::AdmitReq>,
    admit_st: &mut AdmitUiState,
    queue_items: &mut Vec<(i64, String)>,
    pending_images: &mut Vec<(String, String)>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    pending_task_pick: &mut Option<super::app_switch::PendingSwitch>,
    turn: u64,
) {
    // Nothing to append into: flash instead of silently doing nothing (the
    // detail view still closes, but no switch is staged).
    if subtask_ui.main_task_id.is_none() {
        *mode_flash = Some(("当前会话没有主任务，无法追加".into(), anim_tick));
        return;
    }
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
        turn,
    )
    .await
    {
        subtask_ui.main_task_id = None;
        super::app_switch::stage(pending_task_pick, selection, super::app_switch::plain(pick));
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
    // Cancel completions land BEFORE watcher deliveries: a terminal status
    // settles the registry locally so a late (404-driven) `error` delivery
    // can never overwrite it.
    if let Some(view) = subtask_view.as_mut() {
        if let Some((id, result)) = view.poll_cancel() {
            match result {
                Ok(outcome) => {
                    match outcome
                        .status
                        .filter(|status| crate::subtask_registry::is_terminal(status.as_str()))
                    {
                        Some(status) => {
                            view.apply_status(&id, status.as_str());
                            subtask_ui
                                .settle_local(
                                    store,
                                    session_states,
                                    session_id,
                                    chat,
                                    session_id,
                                    &id,
                                    status.as_str(),
                                )
                                .await;
                            let flash = format!("subtask {id} {}", status.as_str());
                            *mode_flash = Some((flash, anim_tick));
                            dirty = true;
                            render = true;
                        }
                        None => {
                            *mode_flash = Some(("已请求取消 subtask".into(), anim_tick));
                        }
                    }
                }
                Err(error) => *mode_flash = Some((format!("{error:#}"), anim_tick)),
            }
        }
    }
    // Subtask watcher deliveries (dedicated channel; the session worker
    // channel is quit on every switch). Locally settled ids are skipped
    // entirely: no view update, no registry write, no flash.
    for ev in subtask_ui.drain_updates() {
        if let UiEvent::SubtaskUpdate {
            ref session,
            ref id,
            ref status,
            ref summary,
            tokens,
        } = ev
        {
            if subtask_ui.is_locally_settled(id) {
                continue;
            }
            if let Some(view) = subtask_view.as_mut() {
                view.apply_update(id, status, summary.as_deref(), tokens);
                dirty = true;
            }
            if let Some((done, total, _)) = crate::subtask_events::apply_update(
                store,
                session_states,
                session_id,
                chat,
                session,
                id,
                status,
                summary.as_deref(),
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
                if let Ok(registry) =
                    crate::remote::subtasks::load_registry(store, session_id).await
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
    // Open detail view: snapshot refresh (merges into the view itself).
    if let Some(view) = subtask_view.as_mut() {
        if let Some(result) = view.poll_refresh() {
            match result {
                Ok(()) => dirty = true,
                Err(error) => *mode_flash = Some((format!("{error:#}"), anim_tick)),
            }
        }
    }
    (dirty, render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::subtasks::load_registry;
    use crate::subtask_registry::find;
    use crossterm::event::{KeyCode, KeyModifiers};
    use opencoder_core::harness::{HarnessRuntime, SubtaskRef};
    use opencoder_store::{SessionMeta, TursoStore};

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
        let mut selection = None;
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
            &mut selection,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            0,
            &mut pending_task_pick,
            &Config::default(),
            0,
        )
        .await;
        assert!(!consumed);
    }

    /// Esc closes the view; Enter on an agent entry stages the conversation
    /// switch (Resume pick + parent return pointer) and closes it.
    #[tokio::test]
    async fn enter_descends_into_the_selected_child_session() {
        let mut view = Some(SubtaskView::new(vec![entry("agent-a")]));
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        let mut subtask_ui = SubtaskUi::new();
        let mut selection = None;
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
            &mut selection,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            0,
            &mut pending_task_pick,
            &Config::default(),
            0,
        )
        .await;
        assert!(consumed);
        assert!(view.is_none(), "fullscreen view closes on descend");
        let pending = pending_task_pick.expect("descend staged");
        assert!(matches!(pending.pick, TaskPick::Resume(id) if id == "agent-a"));
        assert_eq!(pending.parent_after_switch.as_deref(), Some("parent"));
    }

    /// Descending from `/subtasks` drops any in-flight `/agent` capability
    /// query, so its late result cannot replace the staged descend.
    #[tokio::test]
    async fn enter_descend_clears_pending_agent_selection() {
        let mut view = Some(SubtaskView::new(vec![entry("agent-a")]));
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        let mut subtask_ui = SubtaskUi::new();
        let (_tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<TaskPick>>();
        let mut selection = Some(rx);
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
            &mut selection,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            0,
            &mut pending_task_pick,
            &Config::default(),
            0,
        )
        .await;
        assert!(consumed);
        assert!(
            selection.is_none(),
            "descend drops the in-flight /agent query receiver"
        );
        let pending = pending_task_pick.expect("descend staged");
        assert!(matches!(pending.pick, TaskPick::Resume(id) if id == "agent-a"));
        assert_eq!(pending.parent_after_switch.as_deref(), Some("parent"));
    }

    /// Ctrl+Enter without a parent task flashes guidance instead of staging
    /// a switch: the fullscreen view still closes, the `/agent` query stays
    /// live, and no pick is queued behind the user's back.
    #[tokio::test]
    async fn ctrl_enter_without_parent_flashes_and_does_not_stage_switch() {
        let mut view = Some(SubtaskView::new(vec![entry("agent-a")]));
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut chat = ChatView::default();
        let mut subtask_ui = SubtaskUi::new();
        let (_tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<TaskPick>>();
        let mut selection = Some(rx);
        let (admit_tx, _rx) = mpsc::channel(4);
        let mut admit_st = AdmitUiState::default();
        let mut queue_items = Vec::new();
        let mut pending_images = Vec::new();
        let mut mode_flash = None;
        let mut pending_task_pick = None;
        let consumed = key(
            &mut view,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
            FALLBACK_TERM_H,
            &store,
            "parent",
            &mut chat,
            &mut subtask_ui,
            &mut selection,
            &admit_tx,
            &mut admit_st,
            &mut queue_items,
            &mut pending_images,
            &mut mode_flash,
            7,
            &mut pending_task_pick,
            &Config::default(),
            0,
        )
        .await;
        assert!(consumed);
        assert!(view.is_none(), "fullscreen view still closes");
        assert_eq!(
            mode_flash.as_ref().map(|(text, _)| text.as_str()),
            Some("当前会话没有主任务，无法追加"),
            "the no-parent case is explained instead of ignored"
        );
        assert!(pending_task_pick.is_none(), "no switch is staged");
        assert!(selection.is_some(), "the /agent query is left alone");
        assert!(subtask_ui.main_task_id.is_none());
    }

    /// A terminal watcher delivery folds into the registry and chat counters
    /// and flashes the mode line; `interrupted` counts as terminal.
    #[tokio::test]
    async fn tick_folds_terminal_update_and_flashes() {
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "parent".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .set_harness_runtime(
                "parent",
                &HarnessRuntime {
                    subtasks: vec![SubtaskRef {
                        status: "pending".into(),
                        ..entry("agent-a")
                    }],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut subtask_ui = SubtaskUi::new();
        subtask_ui
            .ui_sender()
            .send(UiEvent::SubtaskUpdate {
                session: "parent".into(),
                id: "agent-a".into(),
                status: "interrupted".into(),
                summary: Some("stopped".into()),
                tokens: 7,
            })
            .await
            .unwrap();
        let mut view = None;
        let mut chat = ChatView::default();
        let mut mode_flash = None;
        let (dirty, render) = tick(
            &mut subtask_ui,
            &mut view,
            &store,
            &mut std::collections::HashMap::new(),
            "parent",
            &mut chat,
            &Config::default(),
            &mut mode_flash,
            9,
        )
        .await;
        assert!(dirty && render, "terminal delivery marks dirty + render");
        let registry = load_registry(&store, "parent").await.unwrap();
        let stored = find(&registry, "agent-a").expect("registry kept the entry");
        assert_eq!(stored.status, "interrupted");
        assert_eq!(stored.tokens, 7);
        assert_eq!(chat.subtask_done, 1);
        assert_eq!(chat.tokens_total, 7);
        assert_eq!(mode_flash, Some(("subtask agent-a interrupted".into(), 9)));
    }

    /// A local cancel settle wins over a later watcher `error` delivery: the
    /// event drain skips the locally settled id (no registry rewrite, no
    /// token fold, no flash).
    #[tokio::test]
    async fn late_watcher_error_does_not_overwrite_local_cancel() {
        let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
        store
            .create_session(&SessionMeta {
                id: "parent".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        store
            .set_harness_runtime(
                "parent",
                &HarnessRuntime {
                    subtasks: vec![SubtaskRef {
                        status: "running".into(),
                        tokens: 3,
                        ..entry("agent-a")
                    }],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut subtask_ui = SubtaskUi::new();
        let mut states = std::collections::HashMap::new();
        let mut chat = ChatView::default();
        let settled = subtask_ui
            .settle_local(
                &store,
                &mut states,
                "parent",
                &mut chat,
                "parent",
                "agent-a",
                "cancelled",
            )
            .await;
        assert!(settled, "running -> cancelled settles locally");
        subtask_ui
            .ui_sender()
            .send(UiEvent::SubtaskUpdate {
                session: "parent".into(),
                id: "agent-a".into(),
                status: "error".into(),
                summary: Some("boom".into()),
                tokens: 0,
            })
            .await
            .unwrap();
        let mut view = None;
        let mut mode_flash = None;
        let (dirty, render) = tick(
            &mut subtask_ui,
            &mut view,
            &store,
            &mut states,
            "parent",
            &mut chat,
            &Config::default(),
            &mut mode_flash,
            9,
        )
        .await;
        assert!(!dirty && !render, "skipped delivery changes nothing");
        let registry = load_registry(&store, "parent").await.unwrap();
        let stored = find(&registry, "agent-a").expect("registry kept the entry");
        assert_eq!(stored.status, "cancelled", "late error is ignored");
        assert_eq!(stored.result_summary, None, "cancelled cleared summary");
        assert_eq!(stored.tokens, 3);
        assert_eq!(chat.tokens_total, 0, "local settle adds no token delta");
        assert_eq!(mode_flash, None, "the watcher error is not flashed");
    }
}
