use super::config::Inventory;
use anyhow::Result;
use axum::{
    extract::State,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use std::sync::Arc;

pub async fn serve(worker: Worker, port: u16, token: String) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let worker = Arc::new(worker);
    let app = router(worker.clone(), token);
    // Runtime units are independent. Publishing and host retirement never
    // signal them. SIGTERM is accepted only when quiescence is proven.
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            loop {
                crate::shutdown_signal().await;
                if worker.can_hibernate().await {
                    break;
                }
                tracing::error!(
                    "runtime termination refused: execution, queue or tool still active"
                );
            }
        })
        .await?;
    Ok(())
}

pub fn router(worker: Arc<Worker>, token: String) -> Router {
    Router::new()
        .route("/inventory", get(inventory))
        .route("/rpc", post(rpc))
        .route("/frames", get(frames))
        .with_state(worker)
        .layer(middleware::from_fn_with_state(
            Arc::new(token),
            authenticate,
        ))
}

pub async fn authenticate(
    State(token): State<Arc<String>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let supplied = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    if supplied != Some(token.as_str()) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

async fn inventory(State(worker): State<Arc<Worker>>) -> Response {
    match inventory_of(&worker).await {
        Ok(inventory) => Json(inventory).into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        )
            .into_response(),
    }
}

pub async fn inventory_of(worker: &Worker) -> Result<Inventory> {
    let indexes = worker.indexes().await?;
    let snapshot = worker.snapshot();
    Ok(Inventory {
        runtime_id: worker.runtime_id().map(str::to_owned),
        build: serde_json::to_value(opencoder_core::version::build_info())?,
        owned_processes: opencoder_session::process::active_owned_processes(),
        registration: worker.registration(),
        snapshot,
        indexes,
        can_hibernate: worker.can_hibernate().await,
    })
}

async fn rpc(
    State(worker): State<Arc<Worker>>,
    Json(operation): Json<NodeOperation>,
) -> Json<RpcReply> {
    Json(worker.handle(operation).await)
}

async fn frames(State(worker): State<Arc<Worker>>) -> Response {
    match worker.brain_frames().await {
        Ok(frames) => Json(frames).into_response(),
        Err(error) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        )
            .into_response(),
    }
}
