//! Key-family builders and the JSON row codec.
//!
//! Physical layout (all keys are UTF-8 bytes; the rdb prefix is empty):
//! - `ses/<id>`                       session root row
//! - `msg/<session>/<seq:020>`        message child (per-session order)
//! - `sev/<session>/<seq:020>`        session event child
//! - `inp/<session>/<seq:020>`        input child (global seq value)
//! - `idx/...`                        secondary indexes (see `index`)
//! - `sub/<task_id>`                  subagent task root row
//! - `twf/<id>`                       todo workflow root row
//! - `tit/<workflow>/<todo_id>`       todo item child
//! - `tev/<workflow>/<seq:020>`       todo event child
//! - `evt/<seq:020>`                 platform event log
//! - `evl/<id>`                       event listener spec
//! - `evw/<execution>`                platform pause wait registration
//! - `evc/<consumer>`                 per-consumer replay cursor
//! - `gate/scheduler`                 global scheduler pause gate
//! - `seq/<family>`                   persisted global counter
//!
//! Identifiers are ULID-style (no `/`), so nesting a child range under the
//! parent prefix is collision-free and `delete_range(parent_prefix, ..)`
//! replaces SQL `ON DELETE CASCADE`.

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Zero-padded width for ordinal key components (fits i64::MAX).
pub const ORD_WIDTH: usize = 20;

/// Encode an ordinal as a zero-padded lexicographically ordered key part.
pub fn pad_ord(n: i64) -> String {
    format!("{n:0>ORD_WIDTH$}")
}

/// Family key builders: one function per logical row/index family.
pub fn platform_event_key(seq: i64) -> Vec<u8> {
    format!("evt/{}", pad_ord(seq)).into_bytes()
}

pub fn platform_event_prefix() -> Vec<u8> {
    b"evt/".to_vec()
}

pub fn platform_listener_key(id: &str) -> Vec<u8> {
    format!("evl/{id}").into_bytes()
}

pub fn platform_listener_prefix() -> Vec<u8> {
    b"evl/".to_vec()
}

pub fn platform_wait_key(execution_id: &str) -> Vec<u8> {
    format!("evw/{execution_id}").into_bytes()
}

pub fn platform_wait_prefix() -> Vec<u8> {
    b"evw/".to_vec()
}

pub fn platform_cursor_key(consumer_id: &str) -> Vec<u8> {
    format!("evc/{consumer_id}").into_bytes()
}

pub fn platform_cursor_prefix() -> Vec<u8> {
    b"evc/".to_vec()
}

pub fn scheduler_gate_key() -> Vec<u8> {
    b"gate/scheduler".to_vec()
}

pub fn session_key(id: &str) -> Vec<u8> {
    format!("ses/{id}").into_bytes()
}

pub fn message_prefix(session_id: &str) -> Vec<u8> {
    format!("msg/{session_id}/").into_bytes()
}

pub fn message_key(session_id: &str, seq: i64) -> Vec<u8> {
    let mut k = message_prefix(session_id);
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

pub fn event_prefix(session_id: &str) -> Vec<u8> {
    format!("sev/{session_id}/").into_bytes()
}

pub fn event_key(session_id: &str, seq: i64) -> Vec<u8> {
    let mut k = event_prefix(session_id);
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

pub fn input_prefix(session_id: &str) -> Vec<u8> {
    format!("inp/{session_id}/").into_bytes()
}

pub fn input_key(session_id: &str, seq: i64) -> Vec<u8> {
    let mut k = input_prefix(session_id);
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

pub fn input_seq_index(seq: i64) -> Vec<u8> {
    format!("idx/inp-seq/{}", pad_ord(seq)).into_bytes()
}

pub fn input_dedup_index(session_id: &str, input_id: &str) -> Vec<u8> {
    format!("idx/inp-dedup/{session_id}/{input_id}").into_bytes()
}

pub fn subagent_key(task_id: &str) -> Vec<u8> {
    format!("sub/{task_id}").into_bytes()
}

pub fn subagent_parent_index(parent_session_id: &str, task_id: &str) -> Vec<u8> {
    format!("idx/sub-parent/{parent_session_id}/{task_id}").into_bytes()
}

pub fn subagent_parent_prefix(parent_session_id: &str) -> Vec<u8> {
    format!("idx/sub-parent/{parent_session_id}/").into_bytes()
}

pub fn subagent_child_index(child_session_id: &str, task_id: &str) -> Vec<u8> {
    format!("idx/sub-child/{child_session_id}/{task_id}").into_bytes()
}

pub fn subagent_child_prefix(child_session_id: &str) -> Vec<u8> {
    format!("idx/sub-child/{child_session_id}/").into_bytes()
}

pub fn workflow_key(id: &str) -> Vec<u8> {
    format!("twf/{id}").into_bytes()
}

pub fn todo_item_prefix(workflow_id: &str) -> Vec<u8> {
    format!("tit/{workflow_id}/").into_bytes()
}

pub fn todo_item_key(workflow_id: &str, todo_id: &str) -> Vec<u8> {
    let mut k = todo_item_prefix(workflow_id);
    k.extend_from_slice(todo_id.as_bytes());
    k
}

pub fn todo_event_prefix(workflow_id: &str) -> Vec<u8> {
    format!("tev/{workflow_id}/").into_bytes()
}

pub fn todo_event_key(workflow_id: &str, seq: i64) -> Vec<u8> {
    let mut k = todo_event_prefix(workflow_id);
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

pub fn seq_counter_key(family: &str) -> Vec<u8> {
    format!("seq/{family}").into_bytes()
}

/// Exclusive upper bound of a prefix range (empty = unbounded/all-0xff).
pub fn prefix_upper(prefix: &[u8]) -> Vec<u8> {
    rdb::store::key_upper_bound(prefix).unwrap_or_default()
}

/// Read one row and decode it as JSON.
pub fn get_json<T: DeserializeOwned>(store: &rdb::store::Store, key: &[u8]) -> Result<Option<T>> {
    match rdb::store::get(store, b"", key).map_err(engine_err)? {
        Some(bytes) => decode(&bytes).map(Some),
        None => Ok(None),
    }
}

/// Decode a stored JSON row value.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).context("decode stored rdb row")
}

/// Encode a row value.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).context("encode rdb row")
}

/// Map rdb's string errors into `anyhow` (single conversion point).
pub fn engine_err(e: String) -> anyhow::Error {
    anyhow!("rdb engine: {e}")
}
