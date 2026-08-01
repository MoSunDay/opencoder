//! Shared test fixtures for the mouse-clipboard interaction tests: a
//! panicking `Store` stub, a body-only hit map, and a marker-based
//! `ChatView` builder so tests are independent of the markdown renderer.

use crate::app_helpers::*;
use async_trait::async_trait;
use opencoder_core::Message;
use opencoder_store::{
    Delivery, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta,
    SessionPatch, SubagentTaskRecord,
};
use ratatui::layout::Rect;

/// Minimal `Store` stub whose every method panics. The mouse-copy code paths
/// tested here never touch the store.
pub struct StubStore;

#[async_trait]
impl opencoder_store::Store for StubStore {
    fn backend_name(&self) -> &'static str {
        "stub"
    }
    async fn create_session(&self, _: &SessionMeta) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn get_session(&self, _: &str) -> anyhow::Result<Option<SessionMeta>> {
        unimplemented!()
    }
    async fn list_sessions(&self, _: &SessionFilter) -> anyhow::Result<Vec<SessionListItem>> {
        unimplemented!()
    }
    async fn update_session(&self, _: &str, _: &SessionPatch) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn delete_session(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn clear_other_sessions(&self, _: &str) -> anyhow::Result<u64> {
        unimplemented!()
    }
    async fn append_message(&self, _: &str, _: &Message) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn append_messages(&self, _: &str, _: &[Message]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn load_messages(&self, _: &str) -> anyhow::Result<Vec<Message>> {
        unimplemented!()
    }
    async fn last_message_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn admit_input(&self, _: &SessionInput) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn pending_inputs(&self, _: &str, _: Delivery) -> anyhow::Result<Vec<SessionInput>> {
        unimplemented!()
    }
    async fn promote_inputs(
        &self,
        _: &str,
        _: i64,
        _: Delivery,
    ) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn promote_next_queued(&self, _: &str) -> anyhow::Result<Option<i64>> {
        unimplemented!()
    }
    async fn claim_next_queue(
        &self,
        _: &str,
    ) -> anyhow::Result<Option<(i64, SessionInput)>> {
        unimplemented!()
    }
    async fn delete_input(&self, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn swap_input_order(&self, _: &str, _: i64, _: i64) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn append_events(&self, _: &[SessionEventRecord]) -> anyhow::Result<Vec<i64>> {
        unimplemented!()
    }
    async fn events_after(&self, _: &str, _: i64) -> anyhow::Result<Vec<SessionEventRecord>> {
        unimplemented!()
    }
    async fn last_event_seq(&self, _: &str) -> anyhow::Result<i64> {
        unimplemented!()
    }
    async fn create_subagent_task(&self, _: &SubagentTaskRecord) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn complete_subagent_task(&self, _: &str, _: &str, _: bool) -> anyhow::Result<()> {
        unimplemented!()
    }
    async fn list_subagent_tasks(
        &self,
        _: &str,
    ) -> anyhow::Result<Vec<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn get_subagent_task(&self, _: &str) -> anyhow::Result<Option<SubagentTaskRecord>> {
        unimplemented!()
    }
    async fn cancel_subagent_task(&self, _: &str) -> anyhow::Result<()> {
        unimplemented!()
    }
}

pub fn empty_hits(body: Rect) -> MouseHits {
    MouseHits {
        jump_btn: None,
        top_btn: None,
        body: Some(body),
        queue_panel: None,
        queue_total: 0,
        queue_btns: Vec::new(),
        thinking_btns: Vec::new(),
        subagent_btns: Vec::new(),
        tool_btns: Vec::new(),
        total_rows: 0,
    }
}

/// Build a ChatView whose flattened lines are exactly the given strings
/// (one Marker block per line), so tests are independent of the markdown
/// renderer.
pub fn view_from_lines(lines: &[&str]) -> ChatView {
    let mut v = ChatView::default();
    for &l in lines {
        v.push_marker(ratatui::text::Line::from(l.to_string()));
    }
    v
}
