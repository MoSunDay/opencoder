use super::{config::RuntimeConfig, Host};
use anyhow::{ensure, Result};
use axum::{
    extract::{Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use opencoder_store::fleet::handoff::RuntimeRecord;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router(host: Arc<Host>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/activate-host", post(activate_host))
        .route("/commit-host", post(commit_host))
        .route("/deployment-signal", post(deployment_signal))
        .route("/runtimes", post(register))
        .route("/runtimes/:id/activate", post(activate))
        .route("/runtimes/:id/hibernate", post(hibernate))
        .route("/servers/:id", post(server))
        .with_state(host.clone())
        .layer(middleware::from_fn_with_state(
            Arc::new(host.token.clone()),
            super::runtime::authenticate,
        ))
}

fn reply(result: Result<Value>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => (
            axum::http::StatusCode::CONFLICT,
            Json(json!({"error":format!("{error:#}")})),
        )
            .into_response(),
    }
}

async fn status(State(host): State<Arc<Host>>) -> Response {
    reply(host.status().await)
}

async fn deployment_signal(
    State(host): State<Arc<Host>>,
    Json(request): Json<super::deployment::Request>,
) -> Response {
    reply(host.signal_deployment(request).await)
}
async fn activate_host(State(host): State<Arc<Host>>) -> Response {
    reply(
        host.activate_host()
            .await
            .map(|epoch| json!({"epoch":epoch,"instance":host.instance})),
    )
}

async fn commit_host(State(host): State<Arc<Host>>) -> Response {
    reply(
        async {
            let _lock = host.store.request_lock("host-epoch", "active").await?;
            let mut current = host
                .store
                .definition("host", "current")
                .await?
                .unwrap_or_default();
            ensure!(
                current["instance"] == host.instance,
                "host is no longer current"
            );
            current["ingress"] = json!(host.instance);
            host.store
                .put_definition("host", "current", &current)
                .await?;
            Ok(current)
        }
        .await,
    )
}
async fn register(State(host): State<Arc<Host>>, Json(runtime): Json<RuntimeRecord>) -> Response {
    reply(
        async {
            let config: RuntimeConfig = serde_json::from_value(runtime.config.clone())?;
            config.validate()?;
            let data_dir = config.data_dir.canonicalize()?;
            ensure!(
                data_dir == config.data_dir,
                "runtime data directory must use its canonical path"
            );
            let _lock = host.store.request_lock("release", "registration").await?;
            for existing in host.store.runtimes().await? {
                if existing.id == runtime.id {
                    continue;
                }
                let old: RuntimeConfig = serde_json::from_value(existing.config)?;
                ensure!(
                    old.endpoint != config.endpoint
                        && old.unit != config.unit
                        && !old.data_dir.starts_with(&config.data_dir)
                        && !config.data_dir.starts_with(&old.data_dir),
                    "runtimes must have disjoint ports, units and data directories"
                );
            }
            host.store.register_runtime(&runtime).await?;
            Ok(json!({"registered":runtime.id}))
        }
        .await,
    )
}
async fn activate(State(host): State<Arc<Host>>, Path(id): Path<String>) -> Response {
    reply(
        async {
            let _lock = host.store.request_lock("release", "activation").await?;
            let runtime = host.runtime(&id).await?;
            let inventory = host.inventory(&runtime, true).await?;
            ensure!(inventory.snapshot.ready, "candidate runtime is not ready");
            // A completed real execution, submitted directly to the candidate, is
            // the readiness evidence; process startup alone is insufficient.
            ensure!(
                inventory
                    .indexes
                    .iter()
                    .any(|i| i.id.starts_with("dag-probe-")
                        && i.status == opencoder_core::fleet::ExecutionStatus::Done),
                "candidate has no completed execution probe"
            );
            host.store.activate_runtime(&id).await?;
            host.store
                .put_definition("runtime_sleep", &id, &Value::Null)
                .await?;
            host.sync_inventory().await?;
            host.changes.send_modify(|n| *n += 1);
            Ok(json!({"active_runtime":id}))
        }
        .await,
    )
}
async fn hibernate(State(host): State<Arc<Host>>, Path(id): Path<String>) -> Response {
    reply(host.hibernate(&id).await.map(|()| json!({"hibernated":id})))
}
async fn server(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Json(mut body): Json<Value>,
) -> Response {
    reply(
        async {
            ensure!(
                opencoder_core::fleet::valid_id(&id),
                "invalid server release id"
            );
            let url = reqwest::Url::parse(
                body["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("server URL required"))?,
            )?;
            ensure!(
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port().is_some(),
                "server URL must be loopback HTTP"
            );
            ensure!(body["enabled"].is_boolean(), "enabled must be boolean");
            body["id"] = json!(id);
            host.store
                .put_definition("release_server", &id, &body)
                .await?;
            Ok(json!({"server":id}))
        }
        .await,
    )
}
