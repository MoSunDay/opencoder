//! Server catalog writes share a process lock with execution admission. The
//! resource owner remains the sole writer of binary pool files and NFS owner.
mod binaries;
mod model;
mod references;
mod retired;
mod transport;

use crate::{api::response, AppState};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use opencoder_core::fleet::RpcReply;
use std::{fs::File, sync::Arc, time::Duration};

pub(crate) async fn read_lock(state: &AppState) -> anyhow::Result<File> {
    tokio::time::timeout(
        Duration::from_secs(5),
        state
            .fleet
            .shared_request_lock("catalog-maintenance", "registry"),
    )
    .await?
}

fn guarded(path: &str) -> bool {
    [
        "/api/dag/",
        "/api/brain/plan-defs",
        "/api/brain/runs",
        "/api/brain/capabilities/",
        "/api/schedules",
        "/api/project/todos",
        "/api/agents",
        "/api/harnesses",
        "/api/resource-admission",
        "/api/teams",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn exclusive(method: &str, path: &str) -> bool {
    (path == "/api/dag/defs" && method == "POST")
        || (path.starts_with("/api/dag/defs/") && method == "DELETE")
        || ((path == "/api/dag/binaries" || path.starts_with("/api/dag/binaries/"))
            && !path.ends_with("/nfs"))
        || (path.starts_with("/api/brain/capabilities/")
            && path.ends_with("/target")
            && method == "PUT")
}

pub(crate) async fn protect(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() == axum::http::Method::GET {
        let path = request.uri().path();
        let detail = ["/api/dag/defs/", "/api/dag/binaries/"]
            .iter()
            .any(|prefix| {
                path.strip_prefix(prefix)
                    .is_some_and(|tail| !tail.contains('/') && tail != "nfs")
            });
        let response = next.run(request).await;
        return if detail {
            transport::with_etag(response).await
        } else {
            response
        };
    }
    if !matches!(
        request.method().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    ) || !guarded(request.uri().path())
    {
        return next.run(request).await;
    }
    let lock = if exclusive(request.method().as_str(), request.uri().path()) {
        tokio::time::timeout(
            Duration::from_secs(5),
            state.fleet.request_lock("catalog-maintenance", "registry"),
        )
        .await
    } else {
        tokio::time::timeout(
            Duration::from_secs(5),
            state
                .fleet
                .shared_request_lock("catalog-maintenance", "registry"),
        )
        .await
    };
    let _lock = match lock {
        Ok(Ok(lock)) => lock,
        Ok(Err(error)) => return response(RpcReply::error(500, error.to_string())),
        Err(_) => return response(RpcReply::error(503, "目录正在更新，请稍后重试")),
    };
    // Keep the guard until the owner acknowledges its atomic mutation.
    match retired::validate(&state, &mut request).await {
        Ok(Some(reply)) => return response(reply),
        Ok(None) => {}
        Err(error) => return response(RpcReply::error(503, format!("引用检查失败: {error:#}"))),
    }
    match binaries::check(&state, &mut request).await {
        Ok(Some(reply)) => response(reply),
        Ok(None) => next.run(request).await,
        Err(error) => response(RpcReply::error(
            503,
            format!("目录检查失败，未执行修改: {error:#}"),
        )),
    }
}

pub(crate) async fn delete_dag(
    state: &AppState,
    name: &str,
    headers: &axum::http::HeaderMap,
) -> RpcReply {
    let result = async {
        let Some(definition) = state.fleet.definition("dag", name).await? else {
            return Ok(RpcReply::error(404, "DAG not found"));
        };
        if let Err(reply) = model::unchanged(headers, &definition) {
            return Ok(reply);
        }
        let found = references::find(state, name, true).await?;
        if !found.is_empty() {
            return Ok(model::conflict(
                "catalog_in_use",
                format!("工作流仍被引用: {}", found.join(", ")),
                serde_json::json!({"references":found}),
            ));
        }
        retired::mark(state, "dag", name).await?;
        state.fleet.delete_definition("dag", name).await?;
        Ok::<_, anyhow::Error>(RpcReply::ok(serde_json::json!({"ok":true})))
    }
    .await;
    result.unwrap_or_else(|error| RpcReply::error(503, format!("引用检查失败，未删除: {error:#}")))
}

pub(crate) async fn duplicate_dag(
    state: &AppState,
    spec: &opencoder_dag::DagSpec,
) -> anyhow::Result<Option<RpcReply>> {
    let content = model::dag_content(&serde_json::to_value(spec)?)?;
    if let Some(reply) = retired::validate_value(state, &content).await? {
        return Ok(Some(reply));
    }
    for existing in state.fleet.definitions("dag").await? {
        if existing["name"] == spec.name {
            continue;
        }
        if model::dag_content(&existing).ok().as_ref() == Some(&content) {
            let name = existing["name"].as_str().unwrap_or_default();
            let url = transport::path("/api/dag/defs/", name);
            return Ok(Some(model::conflict(
                "duplicate_definition",
                format!("已有相同工作流，请复用 {name}"),
                serde_json::json!({"name":name,"url":url}),
            )));
        }
    }
    Ok(None)
}

pub(crate) use retired::validate_value;
