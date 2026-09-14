use opencoder_core::Message;

use crate::types::MessageRow;

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
