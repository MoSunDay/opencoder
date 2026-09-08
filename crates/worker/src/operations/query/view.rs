use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};

pub(super) fn bounded_request_ref(request: &CreateExecution) -> CreateExecution {
    CreateExecution {
        id: request.id.clone(),
        kind: request.kind,
        target: request.target.clone(),
        input: bounded_value_ref(&public_input(&request.input), "request.input"),
        node_id: request.node_id.clone(),
    }
}

pub(super) fn bounded_value_ref(value: &Value, field: &str) -> Value {
    let bytes = serialized_len(value);
    if bytes <= 128 * 1024 {
        value.clone()
    } else {
        json!({
            "omitted":true,
            "field":field,
            "total_bytes":bytes,
            "read_via":"detail_field",
        })
    }
}

fn serialized_len(value: &Value) -> usize {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)
        .map(|_| counter.0)
        .unwrap_or(usize::MAX)
}

pub(super) fn truncate_error_ref(value: &str) -> String {
    const LIMIT: usize = 64 * 1024;
    if value.len() <= LIMIT {
        return value.to_owned();
    }
    let end = value
        .char_indices()
        .map(|(offset, _)| offset)
        .take_while(|offset| *offset <= LIMIT)
        .last()
        .unwrap_or(0);
    format!("{}… [truncated, {} bytes]", &value[..end], value.len())
}

pub(super) fn bounded_reply(body: Value) -> Result<RpcReply> {
    if serde_json::to_vec(&body)?.len() > QUERY_RESPONSE_BYTES {
        return Ok(RpcReply::error(
            413,
            "execution detail exceeds the response byte limit",
        ));
    }
    Ok(RpcReply::ok(body))
}

pub(super) fn public_input(input: &Value) -> Value {
    let mut input = input.clone();
    if let Some(envs) = input.get_mut("envs").and_then(Value::as_object_mut) {
        for value in envs.values_mut() {
            *value = json!("[redacted]");
        }
    }
    input
}
