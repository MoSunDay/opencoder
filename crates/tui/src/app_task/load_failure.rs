//! `switch_session`-level refusal. The `load_session_for_switch` tests above
//! pin the error chain at the loader; this module drives the switch entry the
//! event loop actually calls and proves a failed target load is atomic:
//! nothing of the current task is cancelled, quit, snapshotted, rebuilt or
//! rebound, and the rendered chain still names the switch path and session.

use super::*;
use opencoder_store::{SessionMeta, SessionModelOverrides, TursoStore};

/// Current-task state the event loop lends to `switch_session`, bundled so a
/// refusal can be checked field by field. Plain data: nothing here mutates
/// it, only `switch_session` through [`attempt_switch`] may.
pub(super) struct CurrentTask {
    cmd_tx: mpsc::Sender<UiCmd>,
    evt_rx: mpsc::Receiver<UiEvent>,
    /// Receiving end of `cmd_tx`: a refusal must leave it empty (no `Quit`).
    old_commands: mpsc::Receiver<UiCmd>,
    snapshots: std::collections::HashMap<String, crate::session_ui::SessionUiState>,
    running: bool,
    chat: ChatView,
    history: Vec<String>,
    scroll: u32,
    follow: bool,
    queue_scroll: u32,
    sys_tokens: u64,
    queue_items: Vec<(i64, String)>,
    active_skill: Option<String>,
    active_skill_body: Option<String>,
    session_id: String,
    input: String,
    cursor_idx: usize,
    hist_idx: Option<usize>,
    cancel: CancellationToken,
    turn_cancel: SharedCancel,
    child_runtime: ChildRuntimeHandles,
    skill_handle: Arc<Mutex<Option<String>>>,
    question_hub: Arc<opencoder_session::QuestionHub>,
    sidecar_ask: mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
}

/// Handles and values captured right before a switch attempt; compared after
/// it so a refusal is proven against the live objects, not against constants.
struct Before {
    session_id: String,
    display: crate::session_ui::SessionUiState,
    snapshot_keys: Vec<String>,
    input: String,
    cursor_idx: usize,
    hist_idx: Option<usize>,
    cmd_tx: mpsc::Sender<UiCmd>,
    turn_cancel: SharedCancel,
    child_cancels: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
    skill_handle: Arc<Mutex<Option<String>>>,
    question_hub: Arc<opencoder_session::QuestionHub>,
    sidecar_ask: mpsc::Sender<crate::sidecar_ui::SidecarCmd>,
}

fn overrides() -> SessionModelOverrides {
    SessionModelOverrides {
        shared: Some("old/session-shared".into()),
        act: Some("old/session-act".into()),
        plan: None,
    }
}

/// Config of the live task: a model plus a placeholder credential, so the
/// fixture is a runnable task rather than an unconfigured one.
pub(super) fn live_config() -> Config {
    let mut config = Config {
        model: "old/shared".into(),
        ..Config::default()
    };
    config.provider.api_key = Some("test-key".into());
    config
}

/// A live "current" task with a distinctive value in every display and
/// runtime field, so any leak from a half-finished switch is detectable.
pub(super) fn current_task(initial: &SessionState, store: Arc<dyn Store>) -> CurrentTask {
    let (cmd_tx, old_commands) = mpsc::channel(8);
    let (ui_tx, evt_rx) = mpsc::channel(16);
    let mut chat = ChatView {
        agent: "act".into(),
        model: "old/act".into(),
        shared_model: "old/shared".into(),
        model_overrides: Some(overrides()),
        ..Default::default()
    };
    chat.push_marker(Line::from("old transcript"));
    chat.tokens_total = 321;
    CurrentTask {
        cmd_tx,
        evt_rx,
        old_commands,
        snapshots: std::collections::HashMap::new(),
        running: true,
        chat,
        history: vec!["old history".into()],
        scroll: 4,
        follow: false,
        queue_scroll: 2,
        sys_tokens: 99,
        queue_items: vec![(7, "queued draft".into())],
        active_skill: Some("old-skill".into()),
        active_skill_body: Some("old skill body".into()),
        session_id: initial.id.clone(),
        input: "old draft".into(),
        cursor_idx: 4,
        hist_idx: Some(0),
        cancel: CancellationToken::new(),
        turn_cancel: Arc::new(Mutex::new(CancellationToken::new())),
        child_runtime: ChildRuntimeHandles::from_session(initial),
        skill_handle: initial.skill_prompt.clone(),
        question_hub: initial.question_hub.clone(),
        sidecar_ask: crate::sidecar_ui::spawn_actor(initial, ui_tx, Some(store)),
    }
}

/// One `/task` pick against the live task, with the run-wide fixed inputs.
pub(super) async fn attempt_switch(
    task: &mut CurrentTask,
    pick: crate::task::TaskPick,
    workdir: &Path,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    store: &Arc<dyn Store>,
) -> Result<TaskSwitchState> {
    switch_session(
        pick,
        &TuiOpts::default(),
        &None,
        &mut task.cmd_tx,
        &mut task.evt_rx,
        workdir,
        config,
        client,
        store,
        &mut task.snapshots,
        &mut task.running,
        &mut task.chat,
        &mut task.history,
        &mut task.scroll,
        &mut task.follow,
        &mut task.queue_scroll,
        &mut task.sys_tokens,
        &mut task.queue_items,
        &mut task.active_skill,
        &mut task.active_skill_body,
        &mut task.session_id,
        &mut task.input,
        &mut task.cursor_idx,
        &mut task.hist_idx,
        &mut task.cancel,
        &mut task.turn_cancel,
        &mut task.child_runtime,
        &mut task.skill_handle,
        &mut task.question_hub,
        &mut task.sidecar_ask,
    )
    .await
}

fn snapshot_keys(task: &CurrentTask) -> Vec<String> {
    let mut keys: Vec<String> = task.snapshots.keys().cloned().collect();
    keys.sort();
    keys
}

/// The display half of the task, through the production snapshot shape the
/// switch uses to save/restore per-session UI state.
fn display_of(task: &CurrentTask) -> crate::session_ui::SessionUiState {
    crate::session_ui::SessionUiState::snapshot(
        task.running,
        &task.chat,
        &task.history,
        task.scroll,
        task.follow,
        task.queue_scroll,
        task.sys_tokens,
        &task.queue_items,
        &task.active_skill,
        &task.active_skill_body,
        0,
    )
}

fn capture(task: &CurrentTask) -> Before {
    Before {
        session_id: task.session_id.clone(),
        display: display_of(task),
        snapshot_keys: snapshot_keys(task),
        input: task.input.clone(),
        cursor_idx: task.cursor_idx,
        hist_idx: task.hist_idx,
        cmd_tx: task.cmd_tx.clone(),
        turn_cancel: task.turn_cancel.clone(),
        child_cancels: task.child_runtime.cancels.clone(),
        skill_handle: task.skill_handle.clone(),
        question_hub: task.question_hub.clone(),
        sidecar_ask: task.sidecar_ask.clone(),
    }
}

fn assert_intact(task: &mut CurrentTask, before: &Before) {
    assert_eq!(task.session_id, before.session_id, "current session id");
    assert!(
        display_of(task) == before.display,
        "running flag, transcript, models, overrides, history and scroll state"
    );
    assert_eq!(
        snapshot_keys(task),
        before.snapshot_keys,
        "no half-switch snapshot may be written"
    );
    assert_eq!(task.input, before.input, "composer draft");
    assert_eq!(task.cursor_idx, before.cursor_idx, "composer cursor");
    assert_eq!(task.hist_idx, before.hist_idx, "history index");
    assert!(
        !task.cancel.is_cancelled(),
        "the current task's cancellation token must stay live"
    );
    assert!(
        task.cmd_tx.same_channel(&before.cmd_tx),
        "the command channel must not be rebound"
    );
    assert!(
        matches!(
            task.old_commands.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ),
        "no Quit may reach the current worker"
    );
    assert!(
        Arc::ptr_eq(&task.turn_cancel, &before.turn_cancel)
            && !task
                .turn_cancel
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_cancelled(),
        "turn-cancel handle stays the live, uncancelled one"
    );
    assert!(
        Arc::ptr_eq(&task.child_runtime.cancels, &before.child_cancels),
        "child registries stay bound to the current task"
    );
    assert!(
        Arc::ptr_eq(&task.skill_handle, &before.skill_handle),
        "skill prompt handle"
    );
    assert!(
        Arc::ptr_eq(&task.question_hub, &before.question_hub),
        "question hub"
    );
    assert!(
        task.sidecar_ask.same_channel(&before.sidecar_ask),
        "the sidecar actor must not be replaced"
    );
}

/// Drive one switch that must be refused and prove the refusal is atomic.
/// Every fragment of `expected_chain` must appear in the rendered `{:#}`
/// chain (the switch path with the session id, then the underlying cause),
/// and every current-task handle and display value must be exactly what it
/// was before the attempt. Returns the rendered chain for path-specific
/// follow-up checks.
pub(super) async fn assert_switch_refused(
    task: &mut CurrentTask,
    pick: crate::task::TaskPick,
    workdir: &Path,
    config: &Config,
    client: &Arc<dyn ChatStream>,
    store: &Arc<dyn Store>,
    expected_chain: &[&str],
) -> String {
    let before = capture(task);
    let error = attempt_switch(task, pick, workdir, config, client, store)
        .await
        .err()
        .expect("the switch must be refused");
    let chain = format!("{error:#}");
    for fragment in expected_chain {
        assert!(
            chain.contains(fragment),
            "error chain must contain `{fragment}`: {chain}"
        );
    }
    assert_intact(task, &before);
    chain
}

/// The switch entry resumes through `load_session_for_switch`: an absent
/// target must surface the `task switch resume for session <id>` context and
/// leave the current task fully intact.
#[tokio::test]
async fn failed_local_resume_at_switch_entry_keeps_current_task_intact() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let config = live_config();
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    assert_eq!(initial.id, "current");
    let mut task = current_task(&initial, store.clone());
    assert!(
        store.get_session("absent-target").await.unwrap().is_none(),
        "fixture: the target row must not exist"
    );

    assert_switch_refused(
        &mut task,
        crate::task::TaskPick::Resume("absent-target".into()),
        dir.path(),
        &config,
        &client,
        &store,
        &[
            "task switch resume for session absent-target",
            "session not found: absent-target",
        ],
    )
    .await;
    assert!(
        store.get_session("current").await.unwrap().is_none(),
        "a refused switch must not persist the current task either"
    );
}

/// The remote probe runs before the local resume: a `tui_remote_*` row whose
/// Server binding is missing must fail with the `task switch remote load for
/// session <id>` context, never blame the resume path, and keep the current
/// task intact.
#[tokio::test]
async fn failed_remote_probe_at_switch_entry_keeps_current_task_intact() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "remote-unbound".into(),
            kind: Some("tui_remote_agent".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        store
            .harness_runtime("remote-unbound")
            .await
            .unwrap()
            .is_none(),
        "fixture: the bookmark carries no Server binding"
    );
    let config = live_config();
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut task = current_task(&initial, store.clone());

    let chain = assert_switch_refused(
        &mut task,
        crate::task::TaskPick::Resume("remote-unbound".into()),
        dir.path(),
        &config,
        &client,
        &store,
        &[
            "task switch remote load for session remote-unbound",
            "Remote task bookmark is incomplete",
        ],
    )
    .await;
    assert!(
        !chain.contains("task switch resume for session"),
        "the local resume path must not be blamed: {chain}"
    );
}
