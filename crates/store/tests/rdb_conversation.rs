//! Conversation-domain contracts for the rdb (RocksDB) backend, ported from
//! the SQL backend's suites: session CRUD + cascade deletes, message
//! append/load/ordering, bounded pagination and previews. The per-scenario
//! suites live in `rdb_conversation/`.

#[path = "rdb_conversation/suite_events.rs"]
mod suite_events;
#[path = "rdb_conversation/suite_inputs.rs"]
mod suite_inputs;
#[path = "rdb_conversation/suite_messages.rs"]
mod suite_messages;
#[path = "rdb_conversation/suite_sessions.rs"]
mod suite_sessions;

use opencoder_core::fleet::MessageCursor;
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use opencoder_store::rdb_store::RdbStore;
use opencoder_store::{
    SessionFilter, SessionInput, SessionListItem, SessionMeta, SessionPatch, Store,
};

pub(crate) async fn mem() -> RdbStore {
    RdbStore::open_memory().await.unwrap()
}

pub(crate) fn meta_for(id: &str, now: i64) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("test-model".into()),
        created_at: now,
        updated_at: now,
        ..Default::default()
    }
}

pub(crate) fn queue_input(session: &str, id: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.to_string(),
        session_id: session.into(),
        delivery: opencoder_store::Delivery::Queue,
        prompt: format!("prompt-{id}"),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

pub(crate) fn user_msg(id: &str, text: &str) -> Message {
    Message::user(id, text)
}

pub(crate) fn texts(msgs: &[Message]) -> Vec<String> {
    msgs.iter().map(|m| m.text()).collect()
}

/// Tiny event builder shared with the event suite.
pub(crate) fn event(
    session: &str,
    kind: &str,
    payload_json: &str,
) -> opencoder_store::SessionEventRecord {
    opencoder_store::SessionEventRecord {
        session_id: session.into(),
        kind: serde_json::from_str(&format!("\"{kind}\"")).unwrap(),
        payload: serde_json::from_str(payload_json).unwrap(),
        ts: 1,
        seq: None,
        sse_kind: None,
    }
}
