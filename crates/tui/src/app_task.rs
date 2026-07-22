//! `/task` session-switch helpers extracted from `app.rs`'s `run_app` event
//! loop. Kept in a separate module from `app_loop` so that file stays under the
//! 400-line new-file cap; this module holds the larger `TaskOutcome` arms.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::ChatStream;
use opencoder_session::SessionState;
use opencoder_store::{Delivery, Store};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::sys_tokens_for;
use crate::chat::ChatView;
use crate::task::TaskPicker;
use crate::worker::{gate_clear_all, process_cmd, rebind_session, ClearAllGate, UiCmd, UiEvent};

/// The `TaskOutcome::Pick(pick)` arm: perform a session switch. Builds a new
/// `SessionState` (New or Resume), spawns a fresh worker for it, saves the
/// current session's UI snapshot and restores (or initialises) the target
/// session's, rebuilds the chat transcript, resets input/cursor/history, calls
/// `rebind_session` to swap the live channels, and re-syncs the sticky skill.
///
/// Returns `Result` (not `()`) because the body uses `?` to propagate errors
/// from `resolve_agent` / `resume_and_replay`; the caller propagates with `?`.
/// The outer match's post-arm `continue` stays inline in `run_app`.
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
    scroll: &mut u16,
    follow: &mut bool,
    sys_tokens: &mut u64,
    queue_items: &mut Vec<(i64, String)>,
    active_skill: &mut Option<String>,
    active_skill_body: &mut Option<String>,
    session_id: &mut String,
    input: &mut String,
    cursor_idx: &mut usize,
    hist_idx: &mut Option<usize>,
    cancel: &mut CancellationToken,
    skill_handle: &mut Arc<Mutex<Option<String>>>,
) -> Result<()> {
    // Perform session switch.
    let _ = cmd_tx.send(UiCmd::Quit).await;
    let new_session = match &pick {
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
            sess.model = model_label.clone();
            sess
        }
        crate::task::TaskPick::Resume(id) => {
            let new_config = Config::load(workdir).unwrap_or_else(|_| config.clone());
            let replay_cancel = CancellationToken::new();
            opencoder_session::resume::resume_and_replay(
                store.clone(),
                id,
                new_config,
                client.clone(),
                workdir.to_path_buf(),
                Some(replay_cancel),
            )
            .await?
        }
    };
    let new_session_id = new_session.id.clone();
    *model_label = new_session.model.clone();
    let new_cancel = CancellationToken::new();
    let new_session = new_session.with_cancel(new_cancel.clone());
    let new_skill_handle = new_session.skill_prompt.clone();
    let resumed_messages = if let crate::task::TaskPick::Resume(_) = &pick {
        new_session.messages.clone()
    } else {
        Vec::new()
    };
    let (ntx, nrx) = mpsc::channel::<UiEvent>(512);
    let (n_cmd_tx, mut n_cmd_rx) = mpsc::channel::<UiCmd>(64);
    let session_for_worker = new_session;
    let agent_name_for_tokens = session_for_worker.agent.name.clone();
    let workdir_for_tokens = session_for_worker.working_dir.clone();
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
        crate::task::TaskPick::Resume(_) => {
            crate::session_ui::replay_into_chat(
                &agent_name_for_tokens,
                &resumed_messages,
                store,
                &new_session_id,
            )
            .await
        }
        crate::task::TaskPick::New => ChatView {
            agent: agent_name_for_tokens.clone(),
            ..Default::default()
        },
    };
    // Restore UI interaction state from cache,
    // or initialise fresh for a new session.
    if let Some(st) = restored {
        *history = st.history;
        *scroll = st.scroll;
        *follow = st.follow;
        *sys_tokens = st.sys_tokens;
        chat.steer_items = st.chat.steer_items.clone();
        *queue_items = st.queue_items;
        *active_skill = st.active_skill;
        *active_skill_body = st.active_skill_body;
    } else {
        *scroll = 0;
        *follow = true;
        *sys_tokens = sys_tokens_for(&agent_name_for_tokens, &workdir_for_tokens, None);
        chat.steer_items = store
            .pending_inputs(&new_session_id, Delivery::Steer)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|si| (si.seq.unwrap_or(0), si.prompt))
            .collect();
        *queue_items = store
            .pending_inputs(&new_session_id, Delivery::Queue)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|si| (si.seq.unwrap_or(0), si.prompt))
            .collect();
        *active_skill = None;
        *active_skill_body = None;
    }
    *running = false; // chat rebuilt from store on switch-back
    input.clear();
    *cursor_idx = 0;
    *hist_idx = None;
    rebind_session(
        cmd_tx,
        evt_rx,
        session_id,
        cancel,
        n_cmd_tx,
        nrx,
        new_session_id,
        new_cancel,
    );
    // The freshly-spawned worker starts with no
    // skill prompt; re-sync the sticky skill so a
    // resumed session's active skill actually
    // applies to its turns.
    *skill_handle = new_skill_handle;
    if let Some(body) = &*active_skill_body {
        *skill_handle.lock().unwrap() = Some(body.clone());
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
                Style::default().fg(Color::Yellow),
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
                        Style::default().fg(Color::Green),
                    )));
                }
                Err(e) => {
                    if let Some(p) = task_picker.as_mut() {
                        p.reset_confirmation();
                    }
                    chat.push_marker(Line::from(Span::styled(
                        format!("[/task] clear failed: {e:#}"),
                        Style::default().fg(Color::Red),
                    )));
                }
            }
        }
    }
}
