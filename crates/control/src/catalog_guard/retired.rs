//! A removal marker prevents a later concurrent writer from introducing a
//! reference to the just-removed object. Recreating that exact name permits use
//! again. Markers never change execution history or authentication records.
use crate::AppState;
use axum::{body::Body, extract::Request};
use opencoder_core::fleet::RpcReply;
use serde_json::{json, Value};

pub(super) async fn mark(state: &AppState, kind: &str, name: &str) -> anyhow::Result<()> {
    state
        .fleet
        .put_definition(
            "catalog_retired",
            &format!("{kind}:{name}"),
            &json!({"kind":kind,"name":name,"removed_at":opencoder_core::message::now_ms()}),
        )
        .await
}

pub(super) async fn validate(
    state: &AppState,
    request: &mut Request,
) -> anyhow::Result<Option<RpcReply>> {
    if !matches!(request.method().as_str(), "POST" | "PUT" | "PATCH") {
        return Ok(None);
    }
    let markers = state.fleet.definitions("catalog_retired").await?;
    if markers.is_empty() {
        return Ok(None);
    }
    let path = request.uri().path();
    // Publication may deliberately recreate a removed name. Binary file uploads
    // and unrelated request bodies are not catalog reference declarations.
    if path == "/api/dag/defs" || path.starts_with("/api/dag/binaries") || path.contains("/file") {
        return Ok(None);
    }
    if !request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
    {
        return Ok(None);
    }
    let bytes = axum::body::to_bytes(
        std::mem::replace(request.body_mut(), Body::empty()),
        48 * 1024 * 1024,
    )
    .await?;
    let value = serde_json::from_slice::<Value>(&bytes);
    *request.body_mut() = Body::from(bytes);
    let Ok(value) = value else {
        return Ok(None);
    };
    validate_value(state, &value).await
}

pub(crate) async fn validate_value(
    state: &AppState,
    value: &Value,
) -> anyhow::Result<Option<RpcReply>> {
    let mut pools = None;
    for marker in state.fleet.definitions("catalog_retired").await? {
        let name = marker["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("invalid removal marker"))?;
        let dag = marker["kind"] == "dag";
        let agent = marker["kind"] == "agent";
        if !(if agent {
            super::agents::declares(value, name)
        } else {
            super::model::declares(value, name, dag)
        }) {
            continue;
        }
        let exists = if agent {
            let config = opencoder_core::Config::load(&state.workdir)?;
            let root = config
                .agent
                .agents_dir
                .or_else(opencoder_core::agent::agents_dir);
            let name = name.to_owned();
            tokio::task::spawn_blocking(move || {
                opencoder_core::agent::scope::with_root_sync(root, || {
                    opencoder_core::agent::read_agent_meta(&name).is_some()
                })
            })
            .await?
        } else if dag {
            state.fleet.definition("dag", name).await?.is_some()
        } else {
            if pools.is_none() {
                pools = Some(super::transport::Pools::new(state)?.names().await?);
            }
            pools.as_ref().unwrap().iter().any(|n| n == name)
        };
        if !exists {
            return Ok(Some(super::model::conflict(
                "catalog_removed",
                format!(
                    "引用的{}已删除: {name}",
                    if agent {
                        "Agent"
                    } else if dag {
                        "工作流"
                    } else {
                        "二进制资源"
                    }
                ),
                json!({"name":name,"kind":marker["kind"]}),
            )));
        }
    }
    Ok(None)
}
