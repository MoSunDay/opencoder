use opencoder_core::fleet::RpcReply;
use serde_json::{json, Value};

pub(super) fn digest(value: &Value) -> String {
    // serde_json's default map is ordered, so object key order is immaterial.
    opencoder_core::token_hash(&value.to_string())
}

pub(super) fn dag_content(value: &Value) -> anyhow::Result<Value> {
    let spec = opencoder_dag::decode_spec(value.get("spec").unwrap_or(value))
        .map_err(anyhow::Error::msg)?;
    let mut value = serde_json::to_value(spec)?;
    value
        .as_object_mut()
        .expect("DAG spec object")
        .remove("name");
    Ok(value)
}

fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

pub(super) fn mentions(text: &str, name: &str) -> bool {
    !name.is_empty()
        && text.match_indices(name).any(|(offset, _)| {
            (offset == 0 || !word(text.as_bytes()[offset - 1]))
                && text
                    .as_bytes()
                    .get(offset + name.len())
                    .is_none_or(|b| !word(*b))
        })
}

pub(super) fn references(value: &Value, name: &str, dag: bool) -> bool {
    match value {
        Value::String(text) => {
            mentions(text, name) || (dag && mentions(text, &format!("dag-{name}")))
        }
        Value::Array(values) => values.iter().any(|v| references(v, name, dag)),
        Value::Object(fields) => fields.values().any(|v| references(v, name, dag)),
        _ => false,
    }
}

pub(super) fn declares(value: &Value, name: &str, dag: bool) -> bool {
    match value {
        Value::Object(fields) => {
            if dag
                && (value["kind"] == "dag" || value["executor_kind"] == "dag")
                && (value["target"] == name || value["executor_ref"] == name)
            {
                return true;
            }
            fields.iter().any(|(key, value)| {
                let explicit = if dag {
                    matches!(key.as_str(), "capability_id" | "capability_ids")
                } else {
                    matches!(key.as_str(), "resource" | "command" | "args" | "files")
                };
                (explicit && references(value, name, dag))
                    || (key == "files" && uploaded_files_reference(value, name, dag))
                    || declares(value, name, dag)
                    || (key == "executor_spec"
                        && value
                            .as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .is_some_and(|v| declares(&v, name, dag)))
            })
        }
        Value::Array(values) => values.iter().any(|v| declares(v, name, dag)),
        _ => false,
    }
}

fn uploaded_files_reference(value: &Value, name: &str, dag: bool) -> bool {
    use base64::Engine;
    value.as_array().is_some_and(|files| {
        files.iter().any(|file| {
            file["content_b64"]
                .as_str()
                .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .is_some_and(|text| references(&Value::String(text), name, dag))
        })
    })
}

pub(super) fn conflict(code: &str, message: String, details: Value) -> RpcReply {
    RpcReply {
        status: 409,
        body: json!({"error":message,"code":code,"details":details}),
    }
}

pub(super) fn unchanged(headers: &axum::http::HeaderMap, value: &Value) -> Result<(), RpcReply> {
    let Some(expected) = headers.get(axum::http::header::IF_MATCH) else {
        return Ok(());
    };
    let actual = format!("\"{}\"", digest(value));
    if expected.to_str().ok() != Some(actual.as_str()) {
        return Err(RpcReply::error(412, "内容已变化，请重新获取清理清单"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dag_identity_ignores_only_name_and_management_metadata() {
        let a = json!({"name":"one","steps":[{"name":"run","kind":{"type":"binary","resource":"tool@v1","args":["one"]}}]});
        let mut b = a.clone();
        b["name"] = json!("two");
        assert_eq!(
            dag_content(&a).unwrap(),
            dag_content(&json!({"spec":b,"updated_at":9})).unwrap()
        );
        b["steps"][0]["kind"]["args"] = json!(["two"]);
        assert_ne!(dag_content(&a).unwrap(), dag_content(&b).unwrap());
        b = a.clone();
        b["description"] = json!("different contract");
        assert_ne!(dag_content(&a).unwrap(), dag_content(&b).unwrap());
    }
    #[test]
    fn references_include_pins_paths_and_legacy_capability_ids() {
        for value in [
            json!("tool@v3"),
            json!("/mnt/binaries/tool/v2/binary.bin"),
            json!({"capability_id":"dag-tool"}),
        ] {
            assert!(references(&value, "tool", true));
        }
        assert!(!references(&json!("tool-extra"), "tool", false));
        assert!(!references(&json!({"tool":"unrelated"}), "tool", false));
    }
    #[test]
    fn conditional_delete_rejects_changed_or_weak_etags() {
        let value = json!({"name":"one","updated_at":1});
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "if-match",
            format!("\"{}\"", digest(&value)).parse().unwrap(),
        );
        assert!(unchanged(&headers, &value).is_ok());
        assert_eq!(
            unchanged(&headers, &json!({"name":"one","updated_at":2}))
                .unwrap_err()
                .status,
            412
        );
    }
    #[test]
    fn removal_markers_validate_bindings_without_rejecting_search_or_guidance_text() {
        use base64::Engine;
        assert!(declares(
            &json!({"files":[{"path":"run.sh","content_b64":base64::engine::general_purpose::STANDARD.encode("/mnt/tool/v1/binary.bin") }]}),
            "tool",
            false
        ));
        assert!(declares(&json!({"kind":"dag","target":"old"}), "old", true));
        assert!(declares(
            &json!({"type":"binary","resource":"tool@v2"}),
            "tool",
            false
        ));
        assert!(!declares(
            &json!({"prompt":"do not use old or tool","query":"old"}),
            "old",
            true
        ));
        assert!(!declares(&json!({"name":"old","steps":[]}), "old", true));
    }
}
