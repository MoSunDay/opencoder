use crate::{api::response, AppState};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use opencoder_core::fleet::RpcReply;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) fn card_path(path: &str) -> bool {
    path.strip_prefix("/api/agents/")
        .is_some_and(|name| !name.is_empty() && !name.contains('/'))
}

pub(super) fn references(value: &Value, name: &str) -> bool {
    [
        name.to_owned(),
        format!("agent-{name}"),
        format!("operator-{name}"),
    ]
    .iter()
    .any(|name| super::model::references(value, name, false))
}

pub(super) fn declares(value: &Value, name: &str) -> bool {
    match value {
        Value::Object(fields) => {
            (matches!(
                value["kind"].as_str().or(value["executor_kind"].as_str()),
                Some("agent" | "operator")
            ) && (value["target"] == name || value["executor_ref"] == name))
                || fields.iter().any(|(key, value)| {
                    (matches!(
                        key.as_str(),
                        "agent" | "agent_name" | "capability_id" | "capability_ids" | "files"
                    ) && (references(value, name)
                        || [
                            name.to_owned(),
                            format!("agent-{name}"),
                            format!("operator-{name}"),
                        ]
                        .iter()
                        .any(|name| super::model::uploaded_files_reference(value, name, false))))
                        || declares(value, name)
                        || (key == "executor_spec"
                            && value
                                .as_str()
                                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                                .is_some_and(|value| declares(&value, name)))
                })
        }
        Value::Array(values) => values.iter().any(|value| declares(value, name)),
        _ => false,
    }
}

pub(crate) async fn delete(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let reply = remove(&state, &name, &headers)
        .await
        .unwrap_or_else(|error| {
            RpcReply::error(503, format!("引用检查失败，未完成删除: {error:#}"))
        });
    if reply.status == 200 {
        crate::api_agents::fan_out_reload(&state).await;
    }
    response(reply)
}

async fn remove(state: &AppState, name: &str, headers: &HeaderMap) -> anyhow::Result<RpcReply> {
    if opencoder_core::builtin_agents()
        .iter()
        .any(|agent| agent.name == name)
    {
        return Ok(RpcReply::error(400, "cannot delete a builtin agent"));
    }
    if let Err(error) = opencoder_core::agent::validate_agent_name(name) {
        return Ok(RpcReply::error(400, error));
    }
    let root = opencoder_core::agent::agents_dir();
    let owned = name.to_owned();
    let prepared = tokio::task::spawn_blocking(move || {
        opencoder_core::agent::scope::with_root_sync(root, || {
            let deletion = opencoder_agents::write::deletion::LockedAgentDeletion::prepare(&owned)?;
            let mut meta = deletion.meta.clone();
            meta.references = opencoder_agents::references_snapshot(&meta);
            Ok::<_, std::io::Error>((deletion, json!({"ok":true,"meta":meta,"builtin":false})))
        })
    })
    .await?;
    let (deletion, detail) = match prepared {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RpcReply::error(404, "unknown agent"))
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(reply) = super::model::unchanged(headers, &detail) {
        return Ok(reply);
    }
    let found = super::references::find_agent(state, name).await?;
    if !found.is_empty() {
        return Ok(super::model::conflict(
            "catalog_in_use",
            "Agent 仍被引用".into(),
            json!({"references":found}),
        ));
    }
    super::retired::mark(state, "agent", name).await?;
    tokio::task::spawn_blocking(move || deletion.remove()).await??;
    Ok(RpcReply::ok(json!({"ok":true,"deleted":name})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn removal_markers_match_agent_bindings_without_rejecting_unrelated_text() {
        for value in [
            json!({"kind":"operator","target":"sample"}),
            json!({"executor_kind":"agent","executor_ref":"sample"}),
            json!({"capability_id":"agent-sample"}),
            json!({"executor_spec":"{\"agent\":\"sample\"}"}),
            json!({"files":[{"content_b64":base64::engine::general_purpose::STANDARD.encode("invoke operator-sample")}]}),
        ] {
            assert!(declares(&value, "sample"), "{value}");
        }
        for value in [
            json!({"prompt":"do not use sample","query":"agent-sample"}),
            json!({"name":"sample"}),
            json!({"agent":"sample-extra"}),
        ] {
            assert!(!declares(&value, "sample"), "{value}");
        }
    }
}
