//! Versioned, bounded reads from the execution's owning node; no cached results.
use crate::{api::executions, AppState};
use base64::Engine;
use opencoder_core::fleet::{DetailFieldRequest, NodeOperation, RpcReply, EVENT_CHUNK_BYTES};
use serde::Deserialize;
use serde_json::Value;

const MAX_RESULT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Chunk {
    field: String,
    version: String,
    offset: u64,
    next_offset: u64,
    total_bytes: u64,
    eof: bool,
    bytes_b64: String,
}

fn decode(
    body: Value,
    field: &str,
    offset: u64,
    version: Option<&str>,
) -> Result<(Chunk, Vec<u8>), RpcReply> {
    let chunk: Chunk = serde_json::from_value(body).map_err(|_| {
        RpcReply::error(
            409,
            "node lacks versioned result fields; upgrade its Host or Worker",
        )
    })?;
    if chunk.version.is_empty() || version.is_some_and(|version| version != chunk.version) {
        return Err(RpcReply::error(
            409,
            "result version changed; retry the read",
        ));
    }
    if chunk.total_bytes > MAX_RESULT_BYTES {
        return Err(RpcReply::error(
            413,
            "result exceeds the summary read limit; open its native execution detail",
        ));
    }
    if chunk.bytes_b64.len() > EVENT_CHUNK_BYTES.div_ceil(3) * 4 {
        return Err(RpcReply::error(502, "result chunk exceeds its byte limit"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&chunk.bytes_b64)
        .map_err(|_| RpcReply::error(502, "invalid result chunk encoding"))?;
    if chunk.field != field
        || chunk.offset != offset
        || bytes.len() > EVENT_CHUNK_BYTES
        || chunk.next_offset != offset.saturating_add(bytes.len() as u64)
        || chunk.next_offset > chunk.total_bytes
        || chunk.eof != (chunk.next_offset == chunk.total_bytes)
        || (!chunk.eof && bytes.is_empty())
    {
        return Err(RpcReply::error(502, "invalid result chunk boundaries"));
    }
    Ok((chunk, bytes))
}

pub(super) async fn read(state: &AppState, id: &str, field: &str) -> Result<Value, RpcReply> {
    let mut offset = 0;
    let mut version = None;
    let mut total = None;
    let mut output = Vec::new();
    loop {
        let reply = executions::for_id(state, id, |execution| NodeOperation::DetailField {
            request: DetailFieldRequest {
                execution,
                field: field.into(),
                offset,
            },
        })
        .await;
        if reply.status != 200 {
            return Err(reply);
        }
        let (chunk, bytes) = decode(reply.body, field, offset, version.as_deref())?;
        if total.is_some_and(|total| total != chunk.total_bytes) {
            return Err(RpcReply::error(409, "result size changed; retry the read"));
        }
        total = Some(chunk.total_bytes);
        version = Some(chunk.version);
        offset = chunk.next_offset;
        output.extend(bytes);
        if chunk.eof {
            return serde_json::from_slice(&output)
                .map_err(|_| RpcReply::error(502, "invalid JSON in execution result"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page() -> Value {
        json!({"field":"result","version":"v1","offset":0,"next_offset":2,
            "total_bytes":2,"eof":true,"bytes_b64":"e30="})
    }

    #[test]
    fn complete_chunks_preserve_bytes_and_reject_changed_versions() {
        assert_eq!(decode(page(), "result", 0, None).unwrap().1, b"{}");
        assert_eq!(
            decode(page(), "result", 0, Some("v2")).unwrap_err().status,
            409
        );
    }

    #[test]
    fn malformed_or_nonprogressing_chunks_never_produce_a_summary() {
        for (field, value) in [
            ("field", json!("wrong")),
            ("offset", json!(1)),
            ("next_offset", json!(1)),
            ("eof", json!(false)),
            ("bytes_b64", json!("!")),
        ] {
            let mut page = page();
            page[field] = value;
            assert!(decode(page, "result", 0, None).is_err());
        }
        let mut empty = page();
        empty["bytes_b64"] = json!("");
        empty["next_offset"] = json!(0);
        empty["eof"] = json!(false);
        assert!(decode(empty, "result", 0, None).is_err());
        let mut oversized = page();
        oversized["total_bytes"] = json!(MAX_RESULT_BYTES + 1);
        assert_eq!(
            decode(oversized, "result", 0, None).unwrap_err().status,
            413
        );
    }
}
