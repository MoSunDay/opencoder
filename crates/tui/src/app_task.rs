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
use ratatui::backend::Backend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::sys_tokens_for;
use crate::chat::ChatView;
use crate::task::TaskPicker;
use crate::theme;
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
    terminal: &mut crate::render::Term,
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
    skill_handle: &mut Arc<Mutex<Option<String>>>,
) -> Result<()> {
    // Perform session switch.
    // Cancel the in-flight turn before Quit so the worker sees it promptly
    // instead of blocking until the current LLM stream / tool batch finishes.
    cancel.cancel();
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
            // In-flight subagents are replayed to completion during resume;
            // paint a progress banner so the (potentially slow) replay has
            // visible feedback instead of a frozen pre-switch frame.
            draw_resume_replay_banner(terminal, store, id).await?;
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
        crate::task::TaskPick::Fork(id) => {
            // Clone the selected session (meta + messages) into a fresh id,
            // then resume it like any other stored session so the worker
            // starts with the copied conversation context.
            let new_id = opencoder_session::fork::fork_session(store.as_ref(), id).await?;
            let new_config = Config::load(workdir).unwrap_or_else(|_| config.clone());
            draw_resume_replay_banner(terminal, store, &new_id).await?;
            let replay_cancel = CancellationToken::new();
            opencoder_session::resume::resume_and_replay(
                store.clone(),
                &new_id,
                new_config,
                client.clone(),
                workdir.to_path_buf(),
                Some(replay_cancel),
            )
            .await?
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
    let new_skill_handle = new_session.skill_prompt.clone();
    let resumed_messages = match &pick {
        crate::task::TaskPick::Resume(_) | crate::task::TaskPick::Fork(_) => {
            new_session.messages.clone()
        }
        crate::task::TaskPick::New => Vec::new(),
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
        *queue_scroll = st.queue_scroll;
        *sys_tokens = st.sys_tokens;
        chat.steer_items = st.chat.steer_items.clone();
        *queue_items = st.queue_items;
        *active_skill = st.active_skill;
        *active_skill_body = st.active_skill_body;
    } else {
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
        n_cmd_tx,
        nrx,
        new_session_id,
        new_cancel,
        new_turn_cancel,
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

/// Build the resume-replay banner text for `n` in-flight subagents. Returns
/// `None` (callers skip the banner entirely) when there is nothing to replay.
fn resume_banner_message(n: usize) -> Option<String> {
    if n == 0 {
        return None;
    }
    Some(format!(
        "Resuming session \u{2014} replaying {n} subagent(s)\u{2026}"
    ))
}

/// Paint the banner paragraph centered over the whole frame. Kept generic
/// over the backend so it can be unit-tested with a `TestBackend`; the
/// concrete `Term` only enters at the production call site.
fn render_resume_replay_banner(frame: &mut Frame, msg: &str) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let para = Paragraph::new(Line::from(Span::styled(
        msg.to_string(),
        Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    let h = 3u16.min(area.height);
    let w = (msg.chars().count() as u16 + 4).min(area.width);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    frame.render_widget(para, Rect::new(x, y, w, h));
}

/// Paint a full-screen progress banner before `resume_and_replay` when the
/// target session has in-flight (`Running`) subagents. Replay runs each stuck
/// child to completion, which can take a while; without this banner the screen
/// would sit frozen on the pre-switch frame with no feedback. No-op when the
/// session has no in-flight children (or the store query fails).
pub(crate) async fn draw_resume_replay_banner<B: Backend>(
    terminal: &mut Terminal<B>,
    store: &Arc<dyn Store>,
    session_id: &str,
) -> Result<()> {
    let n = store
        .list_subagent_tasks(session_id)
        .await
        .unwrap_or_default()
        .iter()
        .filter(|t| t.status == SubagentStatus::Running)
        .count();
    let Some(msg) = resume_banner_message(n) else {
        return Ok(());
    };
    terminal.draw(|f| render_resume_replay_banner(f, &msg))?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::Message;
    use opencoder_store::{
        Delivery, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
        SessionPatch, SubagentTaskRecord,
    };
    use ratatui::backend::TestBackend;

    /// Store stub for the banner path: only `list_subagent_tasks` is read by
    /// `draw_resume_replay_banner`; every other method is unreachable and
    /// panics if called (mirrors the other TUI store stubs).
    struct BannerStore {
        running: usize,
        cancelled: usize,
        fail_list: bool,
    }

    fn task_record(status: SubagentStatus, id: &str) -> SubagentTaskRecord {
        SubagentTaskRecord {
            task_id: id.to_string(),
            parent_session_id: "parent".into(),
            child_session_id: format!("child-{id}"),
            parent_message_id: Some("a1".into()),
            agent: "explore".into(),
            prompt: "explore the codebase".into(),
            result: None,
            status,
            ok: None,
            started_at: 0,
            completed_at: None,
        }
    }

    #[async_trait::async_trait]
    impl Store for BannerStore {
        fn backend_name(&self) -> &'static str {
            "banner-stub"
        }
        async fn list_subagent_tasks(&self, _: &str) -> Result<Vec<SubagentTaskRecord>> {
            if self.fail_list {
                anyhow::bail!("boom");
            }
            let mut tasks = Vec::new();
            for i in 0..self.running {
                tasks.push(task_record(SubagentStatus::Running, &format!("run-{i}")));
            }
            for i in 0..self.cancelled {
                tasks.push(task_record(SubagentStatus::Cancelled, &format!("cancelled-{i}")));
            }
            Ok(tasks)
        }
        async fn create_session(&self, _: &SessionMeta) -> Result<()> {
            unimplemented!()
        }
        async fn get_session(&self, _: &str) -> Result<Option<SessionMeta>> {
            unimplemented!()
        }
        async fn list_sessions(&self, _: &SessionFilter) -> Result<Vec<SessionListItem>> {
            unimplemented!()
        }
        async fn update_session(&self, _: &str, _: &SessionPatch) -> Result<()> {
            unimplemented!()
        }
        async fn delete_session(&self, _: &str) -> Result<()> {
            unimplemented!()
        }
        async fn clear_other_sessions(&self, _: &str) -> Result<u64> {
            unimplemented!()
        }
        async fn append_message(&self, _: &str, _: &Message) -> Result<i64> {
            unimplemented!()
        }
        async fn append_messages(&self, _: &str, _: &[Message]) -> Result<Vec<i64>> {
            unimplemented!()
        }
        async fn load_messages(&self, _: &str) -> Result<Vec<Message>> {
            unimplemented!()
        }
        async fn last_message_seq(&self, _: &str) -> Result<i64> {
            unimplemented!()
        }
        async fn admit_input(&self, _: &SessionInput) -> Result<i64> {
            unimplemented!()
        }
        async fn pending_inputs(&self, _: &str, _: Delivery) -> Result<Vec<SessionInput>> {
            unimplemented!()
        }
        async fn promote_inputs(&self, _: &str, _: i64, _: Delivery) -> Result<Vec<i64>> {
            unimplemented!()
        }
        async fn promote_next_queued(&self, _: &str) -> Result<Option<i64>> {
            unimplemented!()
        }
        async fn claim_next_queue(&self, _: &str) -> Result<Option<(i64, SessionInput)>> {
            unimplemented!()
        }
        async fn delete_input(&self, _: i64) -> Result<()> {
            unimplemented!()
        }
        async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> Result<()> {
            unimplemented!()
        }
        async fn append_events(&self, _: &[SessionEventRecord]) -> Result<Vec<i64>> {
            unimplemented!()
        }
        async fn events_after(&self, _: &str, _: i64) -> Result<Vec<SessionEventRecord>> {
            unimplemented!()
        }
        async fn last_event_seq(&self, _: &str) -> Result<i64> {
            unimplemented!()
        }
        async fn create_subagent_task(&self, _: &SubagentTaskRecord) -> Result<()> {
            unimplemented!()
        }
        async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> Result<()> {
            unimplemented!()
        }
        async fn get_subagent_task(&self, _: &str) -> Result<Option<SubagentTaskRecord>> {
            unimplemented!()
        }
        async fn cancel_subagent_task(&self, _: &str) -> Result<()> {
            unimplemented!()
        }
    }

    /// Concatenate every cell's symbol row-by-row into a searchable string.
    fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area;
        let mut s = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    /// True when the frame buffer is untouched (all-blank cells).
    fn blank_buffer(terminal: &Terminal<TestBackend>) -> bool {
        let buf = terminal.backend().buffer();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() != " " {
                    return false;
                }
            }
        }
        true
    }

    // ── resume_banner_message (pure) ────────────────────────────────────

    #[test]
    fn resume_banner_message_none_for_zero() {
        assert_eq!(resume_banner_message(0), None, "0 in-flight -> no banner");
    }

    #[test]
    fn resume_banner_message_counts_subagents() {
        let one = resume_banner_message(1).expect("n>0 yields a message");
        assert!(one.contains("replaying 1 subagent"), "got: {one}");
        assert!(one.contains("Resuming session"), "got: {one}");
        let three = resume_banner_message(3).expect("n>0 yields a message");
        assert!(three.contains("replaying 3 subagent"), "got: {three}");
    }

    // ── draw_resume_replay_banner (store stub + TestBackend) ────────────

    #[tokio::test]
    async fn resume_banner_drawn_when_running_subagents() {
        let store: Arc<dyn Store> = Arc::new(BannerStore {
            running: 2,
            cancelled: 1,
            fail_list: false,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        draw_resume_replay_banner(&mut terminal, &store, "parent")
            .await
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(text.contains("Resuming session"), "banner missing; got: {text:?}");
        assert!(
            text.contains("replaying 2 subagent(s)"),
            "running count must appear; got: {text:?}"
        );
    }

    #[tokio::test]
    async fn resume_banner_noop_without_running_subagents() {
        // Cancelled children are replayed on the next user turn, not eagerly
        // during resume — so they must NOT trigger the replay banner.
        let store: Arc<dyn Store> = Arc::new(BannerStore {
            running: 0,
            cancelled: 2,
            fail_list: false,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        draw_resume_replay_banner(&mut terminal, &store, "parent")
            .await
            .unwrap();
        assert!(
            blank_buffer(&terminal),
            "no banner expected when only cancelled tasks exist"
        );
    }

    #[tokio::test]
    async fn resume_banner_noop_when_store_query_fails() {
        // A store error must degrade to a silent no-op, not an error frame
        // mid-switch (mirrors `unwrap_or_default` in the production path).
        let store: Arc<dyn Store> = Arc::new(BannerStore {
            running: 1,
            cancelled: 0,
            fail_list: true,
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        draw_resume_replay_banner(&mut terminal, &store, "parent")
            .await
            .unwrap();
        assert!(
            blank_buffer(&terminal),
            "store failure must degrade to no banner"
        );
    }

    #[test]
    fn banner_renders_within_narrow_area() {
        // Width/height clamping must keep the paragraph inside a tiny frame
        // without panicking or spilling past the buffer edge.
        let mut terminal = Terminal::new(TestBackend::new(10, 2)).unwrap();
        terminal
            .draw(|f| {
                render_resume_replay_banner(f, "Resuming session \u{2014} replaying 9 subagent(s)\u{2026}");
            })
            .unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("Resuming"),
            "prefix must survive width clamping; got: {text:?}"
        );
    }
}
