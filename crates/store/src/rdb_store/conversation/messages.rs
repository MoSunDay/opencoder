//! Messages (`msg/<session>/<seq:020>`): append, load, per-session seqs.
//!
//! The stored row mirrors the SQL columns (id/role/agent/model/blocks/
//! usage/created_at/synthetic/display/provider_state). Per-session seqs
//! are contiguous from 1 (allocated under the write lease from the key
//! range itself), preserving the ordering guarantees the SQL backend's
//! tests assert (`load_message_rows` true seqs, `load_messages_after`
//! positional skips, `last_message_seq`).

use anyhow::{bail, Result};
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv::{decode, message_key, message_prefix};
use crate::rdb_store::{seq, tx};
use crate::types::MessageRow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub blocks: Vec<ContentBlock>,
    #[serde(default)]
    pub usage: MessageUsage,
    pub created_at: i64,
    #[serde(default)]
    pub synthetic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<opencoder_core::ProviderState>,
}

impl MessageRecord {
    pub fn from_message(m: &Message) -> Self {
        MessageRecord {
            id: m.id.clone(),
            role: role_str(m.role).to_string(),
            agent: m.agent.clone(),
            model: m.model.clone(),
            blocks: m.blocks.clone(),
            usage: m.usage.clone(),
            created_at: m.created_at,
            synthetic: m.synthetic,
            display: m.display.clone(),
            provider_state: m.provider_state.clone(),
        }
    }

    pub fn to_message(&self) -> Message {
        Message {
            provider_state: self.provider_state.clone(),
            id: self.id.clone(),
            role: parse_role(&self.role),
            blocks: self.blocks.clone(),
            model: self.model.clone(),
            agent: self.agent.clone(),
            usage: self.usage.clone(),
            created_at: self.created_at,
            synthetic: self.synthetic,
            display: self.display.clone(),
        }
    }
}

pub fn role_str(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

pub fn parse_role(s: &str) -> Role {
    match s {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::User,
    }
}

/// Append a batch atomically; returns the assigned per-session seqs.
/// A non-empty batch into a missing session rejects with the SQL
/// FOREIGN KEY constraint failure (replicated contract).
pub fn append_many(
    store: &rdb::store::Store,
    session_id: &str,
    msgs: &[Message],
) -> Result<Vec<i64>> {
    if !msgs.is_empty() && super::sessions::get(store, session_id)?.is_none() {
        bail!("insert message: FOREIGN KEY constraint failed");
    }
    let prefix = message_prefix(session_id);
    let mut batch = rocksdb::WriteBatch::default();
    let mut seqs = Vec::with_capacity(msgs.len());
    let start = seq::next_child(store, &prefix);
    for (next, m) in (start..).zip(msgs) {
        tx::put(
            &mut batch,
            &message_key(session_id, next),
            &MessageRecord::from_message(m),
        )?;
        seqs.push(next);
    }
    // Activity touch: appending IS activity; keep updated_at monotonic.
    if let Some(last_ts) = msgs.iter().map(|m| m.created_at).max() {
        super::sessions::touch_activity(store, &mut batch, session_id, last_ts)?;
    }
    tx::commit(store, batch)?;
    Ok(seqs)
}

/// All messages of a session in seq order.
pub fn load(store: &rdb::store::Store, session_id: &str) -> Result<Vec<Message>> {
    let mut out = Vec::new();
    for (_, value) in tx::scan_prefix(store, &message_prefix(session_id))? {
        out.push(decode::<MessageRecord>(&value)?.to_message());
    }
    Ok(out)
}

/// Tail after skipping the first `skip_count` rows (keys are skipped
/// without decoding their payloads, mirroring the SQL `OFFSET` win).
pub fn load_after(
    store: &rdb::store::Store,
    session_id: &str,
    skip_count: i64,
) -> Result<Vec<Message>> {
    let skip = skip_count.max(0) as usize;
    let prefix = message_prefix(session_id);
    let keys = rdb::store::prefix_keys_collect(store, &prefix, 0);
    let mut out = Vec::new();
    for key in keys.into_iter().skip(skip) {
        let value = rdb::store::get(store, b"", &key)
            .map_err(kv_err)?
            .unwrap_or_default();
        out.push(decode::<MessageRecord>(&value)?.to_message());
    }
    Ok(out)
}

/// Highest per-session seq (0 when the session has no messages).
pub fn last_seq(store: &rdb::store::Store, session_id: &str) -> Result<i64> {
    Ok(seq::max_child(store, &message_prefix(session_id)))
}

/// Raw relay rows: true per-session seq + raw blocks JSON value.
pub fn load_rows(store: &rdb::store::Store, session_id: &str) -> Result<Vec<MessageRow>> {
    let prefix = message_prefix(session_id);
    let mut out = Vec::new();
    for (key, value) in tx::scan_prefix(store, &prefix)? {
        let Some(s) = seq::parse_ord(&key, &prefix) else {
            continue;
        };
        let rec: MessageRecord = decode(&value)?;
        out.push(MessageRow {
            seq: s,
            role: rec.role,
            blocks: serde_json::to_value(&rec.blocks)?,
            created_at: rec.created_at,
        });
    }
    Ok(out)
}

/// Update one message's usage by (session, message id); exactly one row
/// must match or the update errors out.
pub fn set_usage(
    store: &rdb::store::Store,
    session_id: &str,
    message_id: &str,
    usage: &MessageUsage,
) -> Result<()> {
    let prefix = message_prefix(session_id);
    let mut hits: Vec<(Vec<u8>, MessageRecord)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &prefix)? {
        let rec: MessageRecord = decode(&value)?;
        if rec.id == message_id {
            hits.push((key, rec));
        }
    }
    if hits.len() != 1 {
        bail!("message missing or duplicated while recording harness usage");
    }
    let (key, mut rec) = hits.remove(0);
    rec.usage = usage.clone();
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &rec)?;
    tx::commit(store, batch)
}

/// Ordinal-keyed lookup helper for pagination cursors.
pub(crate) fn record_by_seq(
    store: &rdb::store::Store,
    session_id: &str,
    s: i64,
) -> Result<Option<MessageRecord>> {
    let key = message_key(session_id, s);
    match rdb::store::get(store, b"", &key).map_err(kv_err)? {
        Some(bytes) => Ok(Some(decode(&bytes)?)),
        None => Ok(None),
    }
}

/// First row with `seq > after` (ascending), if any.
pub(crate) fn first_after(
    store: &rdb::store::Store,
    session_id: &str,
    after: i64,
) -> Result<Option<(i64, MessageRecord)>> {
    let prefix = message_prefix(session_id);
    let from = seq::after_ord(&prefix, after);
    let mut hit = None;
    rdb::store::for_each_from(store, &from, true, &mut |key, value| {
        if let Some(s) = seq::parse_ord(key, &prefix) {
            if let Ok(rec) = decode::<MessageRecord>(value) {
                hit = Some((s, rec));
            }
        }
        false
    })
    .map_err(kv_err)?;
    Ok(hit)
}

fn kv_err(e: String) -> anyhow::Error {
    anyhow::anyhow!("rdb engine: {e}")
}

// ---------------------------------------------------------------------------
// Bounded byte pagination (load_message_page / load_transcript_page).
// ---------------------------------------------------------------------------

use crate::types::{MessageChunkPage, MessageChunkRecord};
use opencoder_core::fleet::MessageCursor;

/// Raw stored value bytes for `load_message_page`: the blocks JSON.
fn blocks_bytes(rec: &MessageRecord) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&rec.blocks)?)
}

/// Raw stored value bytes for `load_transcript_page`: the display JSON
/// (provider-private state is never included).
fn transcript_bytes(rec: &MessageRecord) -> Result<Vec<u8>> {
    let value = serde_json::json!({
        "id": rec.id,
        "role": rec.role,
        "blocks": rec.blocks,
        "agent": rec.agent,
        "model": rec.model,
        "usage": rec.usage,
        "created_at": rec.created_at,
        "synthetic": rec.synthetic,
        "display": rec.display,
    });
    Ok(serde_json::to_vec(&value)?)
}

pub fn load_page(
    store: &rdb::store::Store,
    session_id: &str,
    cursor: MessageCursor,
    chunk_bytes: usize,
    raw_budget: usize,
) -> Result<MessageChunkPage> {
    load_page_value(
        store,
        session_id,
        cursor,
        chunk_bytes,
        raw_budget,
        blocks_bytes,
    )
}

pub fn load_transcript_page(
    store: &rdb::store::Store,
    session_id: &str,
    cursor: MessageCursor,
    chunk_bytes: usize,
    raw_budget: usize,
) -> Result<MessageChunkPage> {
    load_page_value(
        store,
        session_id,
        cursor,
        chunk_bytes,
        raw_budget,
        transcript_bytes,
    )
}

fn load_page_value(
    store: &rdb::store::Store,
    session_id: &str,
    mut cursor: MessageCursor,
    chunk_bytes: usize,
    raw_budget: usize,
    value_of: fn(&MessageRecord) -> Result<Vec<u8>>,
) -> Result<MessageChunkPage> {
    if chunk_bytes == 0 || raw_budget < chunk_bytes {
        bail!("message page budget must contain at least one non-empty chunk");
    }
    let mut chunks = Vec::with_capacity(raw_budget / chunk_bytes);
    let mut used = 0usize;
    while used < raw_budget {
        let take = chunk_bytes.min(raw_budget - used);
        let Some(chunk) = read_chunk(store, session_id, cursor, take, value_of)? else {
            return Ok(MessageChunkPage {
                chunks,
                next_cursor: None,
            });
        };
        let next = chunk.offset + chunk.bytes.len() as u64;
        if chunk.bytes.is_empty() && next < chunk.total_bytes {
            bail!("message chunk query did not advance");
        }
        cursor = if next < chunk.total_bytes {
            MessageCursor {
                seq: chunk.seq,
                offset: next,
            }
        } else {
            MessageCursor {
                seq: chunk.seq,
                offset: 0,
            }
        };
        used += chunk.bytes.len();
        chunks.push(chunk);
        if used == raw_budget {
            break;
        }
    }
    let next_cursor = has_more(store, session_id, cursor)?.then_some(cursor);
    Ok(MessageChunkPage {
        chunks,
        next_cursor,
    })
}

fn read_chunk(
    store: &rdb::store::Store,
    session_id: &str,
    cursor: MessageCursor,
    take: usize,
    value_of: fn(&MessageRecord) -> Result<Vec<u8>>,
) -> Result<Option<MessageChunkRecord>> {
    let (seq, rec) = if cursor.offset == 0 {
        match first_after(store, session_id, cursor.seq)? {
            Some(hit) => hit,
            None => return Ok(None),
        }
    } else {
        match record_by_seq(store, session_id, cursor.seq)? {
            Some(rec) => (cursor.seq, rec),
            None => return Ok(None),
        }
    };
    let value = value_of(&rec)?;
    let total = value.len() as u64;
    let start = (cursor.offset as usize).min(value.len());
    let end = (start + take).min(value.len());
    Ok(Some(MessageChunkRecord {
        seq,
        role: rec.role,
        created_at: rec.created_at,
        offset: cursor.offset,
        total_bytes: total,
        bytes: value[start..end].to_vec(),
    }))
}

fn has_more(store: &rdb::store::Store, session_id: &str, cursor: MessageCursor) -> Result<bool> {
    if cursor.offset > 0 {
        return Ok(true);
    }
    Ok(first_after(store, session_id, cursor.seq)?.is_some())
}
