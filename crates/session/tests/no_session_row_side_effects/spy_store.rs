//! Test harness for the no-session-row-side-effects suite: a counting spy
//! around a real in-memory `LibsqlStore` plus the shared fixtures (mirroring
//! tests/control_cmd.rs / plain_skill_prompt.rs).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::{resolve_agent, Config, Message};
use opencoder_llm::{ChatStream, LlmEvent, Usage};
use opencoder_session::SessionState;
use opencoder_store::{
    Delivery, LibsqlStore, SessionEventRecord, SessionFilter, SessionInput, SessionListItem,
    SessionMeta, SessionPatch, Store, SubagentTaskRecord,
};

pub const SESS: &str = "sess-under-test";

/// Delegates every call to a real `LibsqlStore` and counts `create_session`
/// (the only write whose absence the invariant asserts).
pub struct SpyStore {
    inner: Arc<LibsqlStore>,
    creates: Arc<AtomicUsize>,
}

impl SpyStore {
    pub fn creates(&self) -> usize {
        self.creates.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl Store for SpyStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn create_session(&self, m: &SessionMeta) -> Result<()> {
        self.creates.fetch_add(1, Ordering::Relaxed);
        self.inner.create_session(m).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        self.inner.get_session(id).await
    }
    async fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionListItem>> {
        self.inner.list_sessions(f).await
    }
    async fn update_session(&self, id: &str, p: &SessionPatch) -> Result<()> {
        self.inner.update_session(id, p).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        self.inner.delete_session(id).await
    }
    async fn clear_other_sessions(&self, k: &str) -> Result<u64> {
        self.inner.clear_other_sessions(k).await
    }
    async fn append_message(&self, sid: &str, m: &Message) -> Result<i64> {
        self.inner.append_message(sid, m).await
    }
    async fn append_messages(&self, sid: &str, m: &[Message]) -> Result<Vec<i64>> {
        self.inner.append_messages(sid, m).await
    }
    async fn load_messages(&self, sid: &str) -> Result<Vec<Message>> {
        self.inner.load_messages(sid).await
    }
    async fn last_message_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_message_seq(sid).await
    }
    async fn admit_input(&self, i: &SessionInput) -> Result<i64> {
        self.inner.admit_input(i).await
    }
    async fn pending_inputs(&self, sid: &str, d: Delivery) -> Result<Vec<SessionInput>> {
        self.inner.pending_inputs(sid, d).await
    }
    async fn promote_inputs(&self, sid: &str, s: i64, d: Delivery) -> Result<Vec<i64>> {
        self.inner.promote_inputs(sid, s, d).await
    }
    async fn promote_next_queued(&self, sid: &str) -> Result<Option<i64>> {
        self.inner.promote_next_queued(sid).await
    }
    async fn claim_next_queue(&self, sid: &str) -> Result<Option<(i64, SessionInput)>> {
        self.inner.claim_next_queue(sid).await
    }
    async fn delete_input(&self, id: i64) -> Result<()> {
        self.inner.delete_input(id).await
    }
    async fn swap_input_order(&self, sid: &str, a: i64, b: i64) -> Result<()> {
        self.inner.swap_input_order(sid, a, b).await
    }
    async fn append_events(&self, evs: &[SessionEventRecord]) -> Result<Vec<i64>> {
        self.inner.append_events(evs).await
    }
    async fn events_after(&self, sid: &str, after: i64) -> Result<Vec<SessionEventRecord>> {
        self.inner.events_after(sid, after).await
    }
    async fn last_event_seq(&self, sid: &str) -> Result<i64> {
        self.inner.last_event_seq(sid).await
    }
    async fn create_subagent_task(&self, r: &SubagentTaskRecord) -> Result<()> {
        self.inner.create_subagent_task(r).await
    }
    async fn complete_subagent_task(&self, id: &str, res: &str, ok: bool) -> Result<()> {
        self.inner.complete_subagent_task(id, res, ok).await
    }
    async fn list_subagent_tasks(&self, sid: &str) -> Result<Vec<SubagentTaskRecord>> {
        self.inner.list_subagent_tasks(sid).await
    }
    async fn get_subagent_task(&self, id: &str) -> Result<Option<SubagentTaskRecord>> {
        self.inner.get_subagent_task(id).await
    }
    async fn cancel_subagent_task(&self, id: &str) -> Result<()> {
        self.inner.cancel_subagent_task(id).await
    }
}

/// Spy handle + dyn handle over one shared store. Setup writes may go through
/// either: the invariant compares counter deltas taken AFTER seeding, so
/// fixture rows never pollute the measured window.
pub async fn spy_store() -> (Arc<SpyStore>, Arc<dyn Store>) {
    let inner = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let spy = Arc::new(SpyStore {
        inner,
        creates: Arc::new(AtomicUsize::new(0)),
    });
    let dyn_store: Arc<dyn Store> = spy.clone();
    (spy, dyn_store)
}

pub fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

pub fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

pub async fn seed(store: &Arc<dyn Store>, id: &str, agent: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some(agent.into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// Existing-session shape: the row already exists (see `seed`) and
/// `mark_session_created()` mirrors a session loaded by resume/--session.
/// Without that flag `persist()` would legitimately lazy-create the row on
/// the first recorded message -- not the behavior under test here.
pub fn mk_session(agent: &str, client: Arc<dyn ChatStream>, store: Arc<dyn Store>) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        SESS,
        resolve_agent(agent).unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store)
    .mark_session_created()
}

pub fn mk_queue_input(prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: SESS.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

/// Sorted default-filter parent ids -- the observable "session set".
pub async fn parent_ids(store: &Arc<dyn Store>) -> Vec<String> {
    let mut ids: Vec<String> = store
        .list_sessions(&SessionFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    ids.sort();
    ids
}

// Serializes tests that mutate process-global HOME (skill discovery).
static HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII: point HOME/XDG_CONFIG_HOME at `home` (serialized), restore on drop.
pub struct HomeGuard {
    prev: (Option<std::ffi::OsString>, Option<std::ffi::OsString>),
    _lock: std::sync::MutexGuard<'static, ()>,
}

pub fn lock_home(home: &std::path::Path) -> HomeGuard {
    let _lock = HOME_MUTEX.lock().unwrap();
    let prev = (
        std::env::var_os("HOME"),
        std::env::var_os("XDG_CONFIG_HOME"),
    );
    std::env::set_var("HOME", home);
    std::env::set_var("XDG_CONFIG_HOME", home);
    HomeGuard { prev, _lock }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev.0.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match self.prev.1.take() {
            Some(h) => std::env::set_var("XDG_CONFIG_HOME", h),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
