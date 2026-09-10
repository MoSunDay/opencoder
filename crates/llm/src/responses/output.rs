use crate::CompletedToolCall;
use anyhow::{anyhow, bail, Result};
use serde_json::Value;

pub(super) fn item_kind(item: &Value) -> Result<&str> {
    match item["type"].as_str() {
        Some(kind @ ("message" | "reasoning" | "function_call")) => Ok(kind),
        kind => bail!("unsupported Responses output item: {kind:?}"),
    }
}

pub(super) fn function_call(item: &Value) -> Result<CompletedToolCall> {
    let id = required_str(item, "call_id")?;
    let name = required_str(item, "name")?;
    let arguments = required_str(item, "arguments")?;
    let input: Value = serde_json::from_str(arguments)
        .map_err(|e| anyhow!("invalid arguments for Responses tool `{name}` ({id}): {e}"))?;
    if !input.is_object() {
        bail!("arguments for Responses tool `{name}` must be an object");
    }
    Ok(CompletedToolCall {
        id: id.into(),
        name: name.into(),
        input,
    })
}

pub(super) fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Responses item missing {key}"))
}

pub(super) fn text_parts(item: &Value) -> Result<Vec<(usize, String)>> {
    let field = if item["type"] == "reasoning" {
        "summary"
    } else {
        "content"
    };
    let Some(parts) = item.get(field).and_then(Value::as_array) else {
        if field == "summary" {
            return Ok(vec![]);
        }
        bail!("Responses message missing content");
    };
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let key = match part["type"].as_str() {
                Some("output_text" | "summary_text") => "text",
                Some("refusal") => "refusal",
                kind => bail!("unsupported Responses content part: {kind:?}"),
            };
            let text = part[key]
                .as_str()
                .ok_or_else(|| anyhow!("Responses part missing {key}"))?;
            Ok((i, text.to_owned()))
        })
        .collect()
}
