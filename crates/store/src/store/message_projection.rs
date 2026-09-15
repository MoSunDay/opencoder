use opencoder_core::Message;

use crate::types::MessageRow;

pub(super) fn after(mut msgs: Vec<Message>, skip_count: i64) -> Vec<Message> {
    let skip = (skip_count.max(0) as usize).min(msgs.len());
    msgs.drain(..skip);
    msgs
}

pub(super) fn import_report(count: usize) -> crate::ImportReport {
    crate::ImportReport {
        sessions: if count == 0 { 0 } else { 1 },
        messages: count as u32,
        skipped: 0,
    }
}

/// Reconstruct positional rows for stores without a raw-message implementation.
pub(super) fn message_rows(msgs: Vec<Message>) -> Vec<MessageRow> {
    msgs.into_iter()
        .enumerate()
        .map(|(i, m)| MessageRow {
            seq: i as i64 + 1,
            role: serde_json::to_value(m.role)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "user".into()),
            blocks: serde_json::to_value(&m.blocks).unwrap_or(serde_json::Value::Null),
            created_at: m.created_at,
        })
        .collect()
}
