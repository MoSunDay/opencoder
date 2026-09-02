//! `/task` session-switch helpers extracted from `app.rs`'s `run_app` event
//! loop. Kept in a separate module from `app_loop` so that file stays under the
//! 400-line new-file cap; this module holds the larger `TaskOutcome` arms.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::ChatStream;
use opencoder_session::{SessionState, SharedCancel};
use opencoder_store::{Delivery, Store, SubagentStatus};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::sys_tokens_for;
use crate::chat::ChatView;
use crate::task::TaskPicker;
use crate::theme;
use crate::worker::{
    gate_clear_all, process_cmd, rebind_session, ChildRuntimeHandles, ClearAllGate, UiCmd, UiEvent,
};

/// The `TaskOutcome::Pick(pick)` arm: perform a session switch. Builds a new
/// `SessionState` (New or Resume), spawns a fresh worker for it, saves the
/// current session's UI snapshot and restores (or initialises) the target
/// session's, rebuilds the chat transcript, resets input/cursor/history, calls
/// `rebind_session` to swap the live channels, and re-syncs the sticky skill.
///
/// Switching is a PURE data load: `resume` (no subagent replay) so the switch
/// never blocks on re-running children; pending Running/Cancelled tasks stay
/// as-is and the runner's `replay_cancelled_tasks` picks them up on the next
/// user turn (cancellable, with spinner). A hint marker is pushed instead.
///
/// Returns `Result` (not `()`) because the body uses `?` to propagate errors
/// from `resolve_agent` / `resume`; the caller propagates with `?`.
/// The outer match's post-arm `continue` stays inline in `run_app`.
/// Wire `model` stored on a freshly created `/task` session: the bare model
/// id (no provider prefix) -- the same derivation as `SessionState::new` and
/// resume, so the request `model` string is identical no matter how the
/// session was created. `config` is the live in-memory config, so a
/// session-only `/model` switch carries into the new task.
fn new_task_wire_model(config: &Config) -> String {
    config.model_id().to_string()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn switch_session(
    pick: crate::task::TaskPick,
    cmd_tx: &mut mpsc::Sender<UiCmd>,
    evt_rx: &mut mpsc::Receiver<UiEvent>,
    workdir: &Path,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    store: &Arc<dyn Store>,
    model_label: &mut String,
    session_states: &mut std::collections::HashMap<String, crate::session_ui::SessionUiState>,
    running: &mut bool,
    chat: &mut ChatView,
    history: &mut Vec<String>,
    scroll: &mut u32,
    follow: &mut bool,
    queue_scroll: &mut u32,
    sys_tokens: &mut u64,
    queue_items: &mut Vec<(i64, String)>,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    session_id: &mut String,
    input: &mut String,
    cursor_idx: &mut usize,
    hist_idx: &mut Option<usize>,
    cancel: &mut CancellationToken,
    turn_cancel: &mut SharedCancel,
    child_runtime: &mut ChildRuntimeHandles,
    skill_handle: &mut Arc<Mutex<Option<String>>>,
    question_hub: &mut Arc<opencoder_session::QuestionHub>,
    // Ask sender of the OLD session's sidecar actor. Dropped here (the actor
    // then drains and exits) and replaced by a fresh actor bound to the new
    // session — sidecar history never crosses sessions.
    sidecar_ask: &mut mpsc::Sender<crate::sidecar_ui::SidecarAsk>,
) -> Result<()> {
    // Perform session switch.
    // Cancel the in-flight turn before Quit so the worker sees it promptly
    // instead of blocking until the current LLM stream / tool batch finishes.
    cancel.cancel();
    // try_send, never blocking .await: a full command channel (busy worker)
    // must not stall the UI event loop mid-switch; the old worker exits with
    // its sender half regardless once `rebind_session` swaps the channels.
    let _ = cmd_tx.try_send(UiCmd::Quit);
    let (new_session, pending_replay) = match &pick {
        crate::task::TaskPick::New => {
            let new_session_id = opencoder_session::runner::new_id();
            let new_agent = resolve_agent("act").context("agent")?;
            let new_config = Config::load(workdir).unwrap_or_else(|_| config.clone());
            let mut sess = SessionState::new(
                new_session_id,
                new_agent,
                new_config,
                client.clone(),
                workdir.to_path_buf(),
            )
            .with_store(store.clone());
            sess.model = new_task_wire_model(config);
            (sess, 0)
        }
        crate::task::TaskPick::Resume(id) => {
            let new_config = Config::load(workdir).unwrap_or_else(|_| config.clone());
            // Pure load — NO replay here: replaying each pending child could
            // serially block the UI for minutes (per-child replay timeout is
            // minutes-scale). `replay_cancelled_tasks` on the next user turn
            // owns that work instead.
            load_session_for_switch(store, id, new_config, client, workdir).await?
        }
        crate::task::TaskPick::Fork(id) => {
            // Clone the selected session (meta + messages) into a fresh id,
            // then resume it like any other stored session so the worker
            // starts with the copied conversation context.
            let new_id = opencoder_session::fork::fork_session(store.as_ref(), id).await?;
            let new_config = Config::load(workdir).unwrap_or_else(|_| config.clone());
            load_session_for_switch(store, &new_id, new_config, client, workdir).await?
        }
    };
    let new_session_id = new_session.id.clone();
    *model_label = new_session.config.model.clone();
    let new_cancel = CancellationToken::new();
    let new_session = new_session.with_cancel(new_cancel.clone());
    // Mirror `cancel`: the switched session's parent turn-cancel handle must
    // point at the new session so the TUI `>` steer button can interrupt it.
    let new_turn_cancel = new_session
        .turn_cancel
        .clone()
        .unwrap_or_else(|| Arc::new(Mutex::new(CancellationToken::new())));
    // The new session's question hub becomes the live one: attach it and
    // rebind the app-loop pointer (pending dialogs were cleared by the caller).
    new_session.question_hub.attach();
    *question_hub = new_session.question_hub.clone();
    let new_child_runtime = ChildRuntimeHandles::from_session(&new_session);
    let new_skill_handle = new_session.skill_prompt.clone();
    let resumed_messages = match &pick {
        crate::task::TaskPick::Resume(_) | crate::task::TaskPick::Fork(_) => {
            new_session.messages.clone()
        }
        crate::task::TaskPick::New => Vec::new(),
    };
    let (ntx, nrx) = mpsc::channel::<UiEvent>(crate::worker::UI_EVENT_CAPACITY);
    let (n_cmd_tx, mut n_cmd_rx) = mpsc::channel::<UiCmd>(64);
    // Capture the persisted requirement before `new_session` is moved into
    // the worker task; applied after the transcript rebuild below.
    let new_requirement = new_session.requirement.clone();
    let session_for_worker = new_session;
    let agent_name_for_tokens = session_for_worker.agent.name.clone();
    let workdir_for_tokens = session_for_worker.working_dir.clone();
    // Fresh sidecar actor for the incoming session: drop the old sender (the
    // old actor exits once it drains) and spawn against the NEW session with
    // the NEW worker's event channel, before that channel/session move.
    *sidecar_ask =
        crate::sidecar_ui::spawn_actor(&session_for_worker, ntx.clone(), Some(store.clone()));
    tokio::spawn(async move {
        let mut sess = session_for_worker;
        while let Some(cmd) = n_cmd_rx.recv().await {
            if process_cmd(cmd, &mut sess, &ntx).await {
                break;
            }
        }
    });
    // Save current session's UI state before switching.
    session_states.insert(
        session_id.clone(),
        crate::session_ui::SessionUiState::snapshot(
            *running,
            chat,
            history,
            *scroll,
            *follow,
            *queue_scroll,
            *sys_tokens,
            queue_items,
            active_skill,
            active_skill_body,
        ),
    );
    // Restore or create the target session's UI state.
    let restored = session_states.remove(&new_session_id);
    // Always rebuild the chat transcript from the
    // store on switch-back. A cached snapshot can
    // be stale -- background subagents may have
    // progressed or completed while the session
    // was dormant, so replaying from store
    // ensures the latest state is shown.
    *chat = match &pick {
        crate::task::TaskPick::Resume(_) | crate::task::TaskPick::Fork(_) => {
            crate::session_ui::replay_into_chat(
                &agent_name_for_tokens,
                &resumed_messages,
                store,
                &new_session_id,
                // Floor with the live view's accumulated cost so [tok cost]
                // never regresses across the switch-back replay.
                chat.tokens_total,
            )
            .await
        }
        crate::task::TaskPick::New => ChatView {
            agent: crate::terminal_text::sanitize_single_line(&agent_name_for_tokens).into_owned(),
            ..Default::default()
        },
    };
    // Sidecar focus never survives a switch: the rebuilt transcript belongs to
    // the new session, whose sidecar actor starts with an empty conversation.
    chat.sidecar_focus = false;
    // Restore UI interaction state from cache,
    // or initialise fresh for a new session.
    if let Some(st) = restored {
        *history = st.history;
        *scroll = st.scroll;
        *follow = st.follow;
        *queue_scroll = st.queue_scroll;
        *sys_tokens = st.sys_tokens;
        chat.steer_items = st.chat.steer_items.clone();
        *queue_items = st.queue_items;
        *active_skill = st.active_skill;
        *active_skill_body = st.active_skill_body;
    } else {
        // First visit this run: start from a blank per-session UI state --
        // including composer input history (the cached branch restores it
        // from the snapshot; skipping it here leaked the previous session's
        // history into a freshly opened one).
        *history = Vec::new();
        *scroll = 0;
        *follow = true;
        *queue_scroll = 0;
        *sys_tokens = sys_tokens_for(&agent_name_for_tokens, &workdir_for_tokens, None);
        chat.steer_items = crate::queue_panel::pending_mirror(
            store
                .pending_inputs(&new_session_id, Delivery::Steer)
                .await
                .unwrap_or_default(),
        );
        *queue_items = crate::queue_panel::pending_mirror(
            store
                .pending_inputs(&new_session_id, Delivery::Queue)
                .await
                .unwrap_or_default(),
        );
        *active_skill = None;
        *active_skill_body = None;
    }
    // Pending subagents exist: surface a hint marker instead of blocking on
    // eager replay.
    if let Some(text) = pending_replay_hint(pending_replay) {
        chat.push_marker(Line::from(Span::styled(
            text,
            Style::default().fg(theme::warn_color()),
        )));
    }
    // Restore the annotation from the persisted requirement (mirrors the
    // startup path in app.rs) so reopening /ann doesn't seed-and-overwrite
    // with first_prompt. Placed after the snapshot restore so the persisted
    // requirement always wins; `None` for TaskPick::New correctly leaves
    // the annotation unset.
    chat.annotation_text = new_requirement;
    *running = false; // chat rebuilt from store on switch-back
    input.clear();
    *cursor_idx = 0;
    *hist_idx = None;
    rebind_session(
        cmd_tx,
        evt_rx,
        session_id,
        cancel,
        turn_cancel,
        child_runtime,
        n_cmd_tx,
        nrx,
        new_session_id,
        new_cancel,
        new_turn_cancel,
        new_child_runtime,
    );
    // The freshly-spawned worker starts with no
    // skill prompt; re-sync the sticky skill so a
    // resumed session's active skill actually
    // applies to its turns.
    *skill_handle = new_skill_handle;
    if let Some(body) = &*active_skill_body {
        *skill_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(body.clone());
    }
    Ok(())
}

/// The `TaskOutcome::ClearAll { keep_session_id }` arm: wipe every task
/// session except `keep_session_id`. Refused while a turn / subagent is in
/// flight (the running child session would FK-violate on its next append); on
/// success or failure a marker is pushed to the chat and the picker's session
/// list is refreshed. Returns `()` -- no `?`, no break/continue; the outer
/// match's post-arm `continue` stays inline in `run_app`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_clear_all(
    keep_session_id: String,
    running: bool,
    task_picker: &mut Option<TaskPicker>,
    chat: &mut ChatView,
    store: &Arc<dyn Store>,
) {
    // Refuse while a turn / subagent is in flight: a running
    // subagent's child session is still being written to, and
    // clearing would FK-violate its next append. Retry at idle.
    match gate_clear_all(running) {
        ClearAllGate::SkipRunning => {
            if let Some(p) = task_picker.as_mut() {
                p.reset_confirmation();
            }
            chat.push_marker(Line::from(Span::styled(
                "[task] clear busy \u{2014} retry when idle (subagents still running)",
                Style::default().fg(theme::warn_color()),
            )));
        }
        ClearAllGate::Run => {
            let before = task_picker
                .as_ref()
                .map(|p| p.deletable_count())
                .unwrap_or(0);
            match store.clear_other_sessions(&keep_session_id).await {
                Ok(n) => {
                    let sessions = store
                        .list_sessions(&opencoder_store::SessionFilter::default())
                        .await
                        .unwrap_or_default();
                    if let Some(p) = task_picker.as_mut() {
                        p.reset_sessions(sessions);
                    }
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/task] cleared {n} of {before} task(s)"),
                        Style::default().fg(theme::ok_color()),
                    )));
                }
                Err(e) => {
                    if let Some(p) = task_picker.as_mut() {
                        p.reset_confirmation();
                    }
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/task] clear failed: {e:#}"),
                        Style::default().fg(theme::err_color()),
                    )));
                }
            }
        }
    }
}

/// Pure-data load for a `/task` switch (Resume/Fork): rebuilds the target
/// session via [`opencoder_session::resume::resume`] WITHOUT replaying
/// pending subagents. Pending (Running/Cancelled) children stay untouched —
/// the runner's `replay_cancelled_tasks` picks them up on the next user turn
/// (cancellable, with spinner feedback), so the switch itself never blocks
/// on serial LLM re-runs. Returns the loaded session plus the number of
/// pending tasks for the post-switch hint marker.
pub(crate) async fn load_session_for_switch(
    store: &Arc<dyn Store>,
    id: &str,
    config: Config,
    client: &Arc<dyn ChatStream>,
    workdir: &Path,
) -> Result<(SessionState, usize)> {
    // `list_subagent_tasks` is indexed on (parent_session_id, seq); one
    // cheap query both counts the hint and feeds nothing else — replay is
    // deferred by design.
    let pending = store
        .list_subagent_tasks(id)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                SubagentStatus::Running | SubagentStatus::Cancelled
            )
        })
        .count();
    let session = opencoder_session::resume::resume(
        store.clone(),
        id,
        config,
        client.clone(),
        workdir.to_path_buf(),
    )
    .await?;
    Ok((session, pending))
}

/// Marker line text shown after a switch when `n` subagents are pending
/// replay. `None` (callers skip the marker) when nothing is pending. Echoes
/// the picker's `⊗ N replay pending` badge so both surfaces agree.
pub(crate) fn pending_replay_hint(n: usize) -> Option<String> {
    if n == 0 {
        return None;
    }
    Some(format!(
        "[task] {n} subagent(s) replay pending \u{2014} resume on next message"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::Message;
    use opencoder_llm::MockChatClient;
    use opencoder_store::LibsqlStore;

    // ── pending_replay_hint (pure) ──────────────────────────────────────

    #[test]
    fn new_task_wire_model_strips_provider_prefix() {
        let prefixed = Config {
            model: "prov-x/model-x".into(),
            ..Config::default()
        };
        assert_eq!(new_task_wire_model(&prefixed), "model-x");
        let bare = Config {
            model: "model-x".into(),
            ..Config::default()
        };
        assert_eq!(new_task_wire_model(&bare), "model-x", "already bare");
    }

    #[test]
    fn pending_replay_hint_none_for_zero() {
        assert_eq!(pending_replay_hint(0), None, "no pending -> no marker");
    }

    #[test]
    fn pending_replay_hint_lists_count_and_trigger() {
        let one = pending_replay_hint(1).expect("n>0 yields a hint");
        assert!(one.contains("1 subagent(s)"), "got: {one}");
        assert!(one.contains("replay pending"), "got: {one}");
        assert!(one.contains("next message"), "got: {one}");
        let three = pending_replay_hint(3).expect("n>0 yields a hint");
        assert!(three.contains("3 subagent(s)"), "got: {three}");
    }

    // ── load_session_for_switch (pure load, no replay) ──────────────────

    fn user_msg(id: &str, text: &str) -> Message {
        Message {
            id: id.into(),
            role: opencoder_core::Role::User,
            blocks: vec![opencoder_core::ContentBlock::text(text)],
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        }
    }

    fn assistant_task_use(id: &str, tool_use_id: &str) -> Message {
        Message {
            id: id.into(),
            role: opencoder_core::Role::Assistant,
            blocks: vec![opencoder_core::ContentBlock::ToolUse {
                id: tool_use_id.into(),
                name: "task".into(),
                input: serde_json::json!({"prompt": "explore"}),
            }],
            model: None,
            agent: None,
            usage: opencoder_core::MessageUsage::default(),
            created_at: 0,
            synthetic: false,
        }
    }

    /// The switch path must be a PURE data load: a Cancelled subagent stays
    /// Cancelled (no eager LLM replay), its dangling `task` tool_use stays
    /// dangling in the parent transcript (no synthetic error tool_result —
    /// the next-turn replay will answer it), child messages are untouched,
    /// and the pending count is reported for the hint marker.
    #[tokio::test]
    async fn load_session_for_switch_is_pure_load_no_replay() {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(
            LibsqlStore::open(dir.path().join("switch.db"))
                .await
                .unwrap(),
        );
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());

        // Parent session: user prompt + assistant dangling `task` tool_use.
        for sid in ["parent", "child-x"] {
            store
                .create_session(&opencoder_store::SessionMeta {
                    id: sid.into(),
                    title: Some(sid.into()),
                    agent: Some("act".into()),
                    model: Some("m".into()),
                    created_at: 0,
                    updated_at: 0,
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        store
            .append_messages(
                "parent",
                &[
                    user_msg("u1", "explore the repo"),
                    assistant_task_use("a1", "task-1"),
                ],
            )
            .await
            .unwrap();
        store
            .append_message("child-x", &user_msg("c-u1", "child working"))
            .await
            .unwrap();
        store
            .create_subagent_task(&opencoder_store::SubagentTaskRecord {
                task_id: "task-1".into(),
                parent_session_id: "parent".into(),
                child_session_id: "child-x".into(),
                parent_message_id: Some("a1".into()),
                agent: "explore".into(),
                prompt: "explore the repo".into(),
                result: None,
                status: opencoder_store::SubagentStatus::Cancelled,
                ok: None,
                started_at: 0,
                completed_at: None,
            })
            .await
            .unwrap();

        let before_parent = store.load_messages("parent").await.unwrap();
        let before_child = store.load_messages("child-x").await.unwrap();

        let (session, pending) =
            load_session_for_switch(&store, "parent", Config::default(), &client, dir.path())
                .await
                .unwrap();

        // Pending count feeds the hint marker.
        assert_eq!(pending, 1, "one cancelled task must be reported pending");

        // Task untouched: still Cancelled, no result backfilled.
        let task = store.get_subagent_task("task-1").await.unwrap().unwrap();
        assert_eq!(
            task.status,
            opencoder_store::SubagentStatus::Cancelled,
            "switch must not replay the cancelled task"
        );
        assert!(task.result.is_none(), "no replay result may be backfilled");

        // Parent transcript unchanged: the dangling tool_use is still the
        // last word (no synthetic Tool message answering it on load).
        let after_parent = store.load_messages("parent").await.unwrap();
        assert_eq!(
            after_parent.len(),
            before_parent.len(),
            "pure load must not append messages to the parent"
        );
        let dangling_kept = session
            .messages
            .last()
            .map(|m| {
                matches!(m.role, opencoder_core::Role::Assistant)
                    && m.blocks
                        .iter()
                        .any(|b| matches!(b, opencoder_core::ContentBlock::ToolUse { id, .. } if id == "task-1"))
            })
            .unwrap_or(false);
        assert!(
            dangling_kept,
            "replayable dangling tool_use must survive the load unanswered"
        );

        // Child transcript unchanged.
        let after_child = store.load_messages("child-x").await.unwrap();
        assert_eq!(
            after_child.len(),
            before_child.len(),
            "pure load must not touch the child transcript"
        );
    }

    /// No pending tasks -> count 0 (no marker); Running tasks count as
    /// pending too, and Completed ones do not.
    #[tokio::test]
    async fn load_session_for_switch_counts_only_pending_statuses() {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn Store> = Arc::new(
            LibsqlStore::open(dir.path().join("counts.db"))
                .await
                .unwrap(),
        );
        let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        store
            .create_session(&opencoder_store::SessionMeta {
                id: "p".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let mk = |task_id: &str, child: &str, status| opencoder_store::SubagentTaskRecord {
            task_id: task_id.into(),
            parent_session_id: "p".into(),
            child_session_id: child.into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "p".into(),
            result: None,
            status,
            ok: None,
            started_at: 0,
            completed_at: None,
        };
        for sid in ["c1", "c2", "c3"] {
            store
                .create_session(&opencoder_store::SessionMeta {
                    id: sid.into(),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        store
            .create_subagent_task(&mk("t1", "c1", opencoder_store::SubagentStatus::Running))
            .await
            .unwrap();
        store
            .create_subagent_task(&mk("t2", "c2", opencoder_store::SubagentStatus::Cancelled))
            .await
            .unwrap();
        store
            .create_subagent_task(&mk("t3", "c3", opencoder_store::SubagentStatus::Completed))
            .await
            .unwrap();

        let (_session, pending) =
            load_session_for_switch(&store, "p", Config::default(), &client, dir.path())
                .await
                .unwrap();
        assert_eq!(
            pending, 2,
            "Running + Cancelled count; Completed is terminal and must not"
        );
    }
}
