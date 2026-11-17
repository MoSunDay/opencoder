//! Off-loop `TranscriptReset` rebuild: the UI event fold must never block on
//! the (potentially seconds-long) view rebuild. It spawns the rebuild
//! (`reset_rebuild`), leaves every post-reset event queued in the bounded
//! channel, and the loop installs the rebuilt view when it lands — then (and
//! only then) folds the queued events against the fresh view.

use super::*;

use crate::chat::ChatBlock;
use anyhow::Result;
use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{
    Delivery, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionPatch, Store, SubagentTaskRecord, TursoStore,
};
use std::time::{Duration, Instant};

/// Delegates everything to an inner memory store EXCEPT `get_session`,
/// which sleeps first. `replay_into_chat_inner` opens with `get_session`,
/// so the sleep pins the WHOLE rebuild in a slow window — the same shape a
/// big session (meta + subagent reads + image prefetch) produces.
struct DelayStore(Arc<TursoStore>, Duration);

#[async_trait::async_trait]
impl Store for DelayStore {
    fn backend_name(&self) -> &'static str {
        self.0.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<()> {
        self.0.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        tokio::time::sleep(self.1).await;
        self.0.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.0.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
        self.0.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.0.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.0.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.0.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
        self.0.append_messages(sid, m).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.0.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.0.last_message_seq(sid).await
    }
    async fn admit_input(&self, i: &SessionInput) -> Result<i64> {
        self.0.admit_input(i).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.0.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, up: i64, d: Delivery) -> Result<Vec<i64>> {
        self.0.promote_inputs(sid, up, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.0.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
        self.0.claim_next_queue(sid).await
    }
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.0.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.0.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, ev: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.0.append_events(ev).await
    }
    async fn events_after(&self, sid: &str, s: i64) -> Result<Vec<SessionEventRecord>> {
        self.0.events_after(sid, s).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.0.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
        self.0.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, r: &str, ok: bool) -> Result<()> {
        self.0.complete_subagent_task(id, r, ok).await
    }
    async fn list_subagent_tasks(&self, pid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.0.list_subagent_tasks(pid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.0.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.0.cancel_subagent_task(id).await
    }
}

/// Shared fold plumbing: one fold call over a fresh loop-local set.
#[allow(clippy::too_many_arguments)]
struct FoldCtx {
    chat: ChatView,
    store: Arc<dyn Store>,
    session_id: &'static str,
    queue_items: Vec<(i64, String)>,
    plan_skill_active: bool,
    admit: crate::queue_admitter::AdmitUiState,
    running: bool,
    cancelled: bool,
    drain_pending: bool,
    skip_next_render: bool,
    follow: bool,
    cmd_tx: mpsc::Sender<UiCmd>,
    cancel: CancellationToken,
    turn_gen: u64,
    evt_tx: mpsc::Sender<UiEvent>,
    evt_rx: mpsc::Receiver<UiEvent>,
    notepad: Option<crate::notepad::NotepadView>,
    question_menu: Option<crate::question_menu::QuestionMenu>,
    question_hub: std::sync::Arc<opencoder_session::QuestionHub>,
    reset: crate::reset_rebuild::ResetRebuildState,
}

impl FoldCtx {
    fn new(chat: ChatView, store: Arc<dyn Store>, session_id: &'static str) -> Self {
        let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>(64);
        Self {
            chat,
            store,
            session_id,
            queue_items: Vec::new(),
            plan_skill_active: false,
            admit: crate::queue_admitter::AdmitUiState::default(),
            running: true,
            cancelled: false,
            drain_pending: false,
            skip_next_render: false,
            follow: true,
            cmd_tx,
            cancel: CancellationToken::new(),
            turn_gen: 0,
            evt_tx,
            evt_rx,
            notepad: None,
            question_menu: None,
            question_hub: std::sync::Arc::new(opencoder_session::QuestionHub::default()),
            reset: crate::reset_rebuild::ResetRebuildState::default(),
        }
    }

    /// Queue an event behind the fold (simulating worker deliveries).
    async fn send_ev(&self, ev: UiEvent) {
        self.evt_tx.send(ev).await.unwrap();
    }

    async fn fold(&mut self, ev: Option<UiEvent>) -> LoopFlow {
        fold_ui_events(
            ev,
            &mut self.chat,
            &self.store,
            self.session_id,
            &mut self.queue_items,
            &mut self.plan_skill_active,
            &mut self.admit,
            &mut self.running,
            &mut self.cancelled,
            &mut self.drain_pending,
            &mut self.skip_next_render,
            &mut self.follow,
            &self.cmd_tx,
            &mut self.cancel,
            &mut self.turn_gen,
            &mut self.evt_rx,
            &mut self.notepad,
            &mut self.question_menu,
            &self.question_hub,
            &mut self.reset,
        )
        .await
    }
}

fn marker_text(b: &ChatBlock) -> Option<String> {
    match b {
        ChatBlock::Marker(lines) => Some(
            lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// The regression: a slow rebuild (300ms store) must not block the fold —
/// `fold_ui_events` returns at once, and a mini UI loop keeps ticking
/// (frames + input) while the rebuild runs. Installing the result drops the
/// pre-reset marker and keeps the remembered turn echo.
#[tokio::test]
async fn fold_returns_and_ticks_flow_while_reset_rebuild_is_blocked() {
    let inner = Arc::new(TursoStore::open_memory().await.unwrap());
    inner
        .create_session(&SessionMeta {
            id: "sess-slow-reset".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let store: Arc<dyn Store> = Arc::new(DelayStore(inner, Duration::from_millis(300)));

    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    chat.blocks
        .push(ChatBlock::Marker(vec![Line::from("stale-pre-reset-marker")]));
    chat.pending_turn_echo = Some("tail echo".into());

    let mut ctx = FoldCtx::new(chat, store, "sess-slow-reset");
    let t0 = Instant::now();
    let flow = ctx
        .fold(Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))))
        .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        t0.elapsed() < Duration::from_millis(150),
        "fold must not block on the rebuild (took {:?})",
        t0.elapsed()
    );
    assert!(ctx.reset.is_pending(), "the rebuild is in flight");
    assert!(
        ctx.chat
            .blocks
            .iter()
            .any(|b| marker_text(b).as_deref().is_some_and(|t| t.contains("stale-pre-reset-marker"))),
        "the old view stays in place until install"
    );

    // Mini UI loop (mirrors app.rs select): frames, input and the rebuild
    // done arm race — the rebuild must not starve any of them.
    let mut ticks = 0u32;
    let mut keys = 0u32;
    let mut ui_tick = tokio::time::interval(Duration::from_millis(10));
    ui_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let (key_tx, mut key_rx) = mpsc::channel::<()>(4);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = key_tx.send(()).await;
    });
    let mut rebuilt: Option<ChatView> = None;
    while rebuilt.is_none() {
        tokio::select! {
            _ = ui_tick.tick() => ticks += 1,
            k = key_rx.recv() => { if k.is_some() { keys += 1; } }
            v = ctx.reset.wait_done("sess-slow-reset") => rebuilt = v,
        }
    }
    assert!(ticks >= 5, "frames kept flowing during the rebuild: {ticks}");
    assert!(keys >= 1, "input kept flowing during the rebuild");
    ctx.chat = rebuilt.expect("the rebuild delivers a view");
    assert!(
        !ctx.chat
            .blocks
            .iter()
            .any(|b| marker_text(b).as_deref().is_some_and(|t| t.contains("stale-pre-reset-marker"))),
        "the pre-reset marker is gone after install"
    );
    assert_eq!(
        ctx.chat.pending_turn_echo.as_deref(),
        Some("tail echo"),
        "the remembered echo survives the rebuild"
    );
}

/// Ordering / losslessness: events queued BEHIND a reset stay in the channel
/// until the rebuilt view is installed; folding them afterwards lands their
/// content below the rebuilt view's user boundary.
#[tokio::test]
async fn events_after_reset_stay_queued_until_install() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-queued-reset".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    chat.pending_turn_echo = Some("post-reset turn".into());

    let mut ctx = FoldCtx::new(chat, store, "sess-queued-reset");
    ctx.send_ev(UiEvent::Session(SessionEvent::LlmRoundStart {
        started_at_ms: 1_000,
    }))
    .await;
    ctx.send_ev(UiEvent::Session(SessionEvent::TextDelta(
        "streamed-after-reset".into(),
    )))
    .await;

    let flow = ctx
        .fold(Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))))
        .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ctx.reset.is_pending());
    // Snapshot the queue (peek-style) so the assertions do not consume it.
    let mut queued = Vec::new();
    while let Ok(ev) = ctx.evt_rx.try_recv() {
        queued.push(ev);
    }
    assert!(
        matches!(
            queued.first(),
            Some(UiEvent::Session(SessionEvent::LlmRoundStart { .. }))
        ),
        "the post-reset round start stays queued, in order"
    );
    assert!(
        matches!(
            queued.get(1),
            Some(UiEvent::Session(SessionEvent::TextDelta(_)))
        ),
        "the post-reset delta stays queued, in order"
    );
    assert_eq!(queued.len(), 2, "nothing else was drained");

    // Install, then fold the queued events in arrival order.
    if let Some(rebuilt) = ctx.reset.wait_done("sess-queued-reset").await {
        ctx.chat = rebuilt;
    }
    let user_idx = ctx
        .chat
        .blocks
        .iter()
        .position(|b| matches!(b, ChatBlock::User { rendered } if rendered
            .iter()
            .any(|l| l.to_string().contains("post-reset turn"))));
    let user_idx = user_idx.expect("the rebuilt view opens with the echo boundary");
    for ev in queued {
        let _ = ctx.fold(Some(ev)).await;
    }
    let assistant_idx = ctx.chat.blocks.iter().position(|b| {
        matches!(b, ChatBlock::Assistant { raw, .. } if raw.contains("streamed-after-reset"))
    });
    let assistant_idx = assistant_idx.expect("the queued delta folded after install");
    assert!(
        assistant_idx > user_idx,
        "post-reset text lands below the rebuilt view's user boundary"
    );
}

/// `/task` switch race: a rebuild spawned for the previous session must be
/// dropped, never installed, and must clear the pending slot.
#[tokio::test]
async fn stale_rebuild_for_other_session_is_dropped() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-original".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut ctx = FoldCtx::new(
        ChatView {
            agent: "act".into(),
            ..Default::default()
        },
        store,
        "sess-original",
    );
    let flow = ctx
        .fold(Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))))
        .await;
    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(ctx.reset.is_pending());

    // The switch already happened: the loop now waits on another session.
    assert!(
        ctx.reset.wait_done("sess-switched-to").await.is_none(),
        "a stale rebuild is dropped, not installed"
    );
    assert!(!ctx.reset.is_pending(), "the pending slot is cleared");
}
