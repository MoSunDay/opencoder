use crate::AppState;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use opencoder_core::fleet::RpcReply;
use std::sync::Arc;

async fn forward(
    state: &AppState,
    base: &str,
    method: reqwest::Method,
    path: &str,
    body: Vec<u8>,
) -> anyhow::Result<Response> {
    let url = reqwest::Url::parse(base)?;
    anyhow::ensure!(
        url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
        "internal service must use loopback HTTP"
    );
    let token = state
        .lifecycle
        .credential
        .get()
        .ok_or_else(|| anyhow::anyhow!("service credential unavailable"))?;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()?
        .request(method, format!("{}{path}", base.trim_end_matches('/')))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await?;
    Ok(Response::builder()
        .status(response.status().as_u16())
        .header("content-type", "application/json")
        .body(axum::body::Body::from(response.bytes().await?))?)
}

pub async fn forward_resources(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if !matches!(
        request.uri().path(),
        "/api/agents/nfs" | "/api/dag/wasm/nfs"
    ) {
        return next.run(request).await;
    }
    let Some(platform) = state.lifecycle.platform.get() else {
        return next.run(request).await;
    };
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let body = match axum::body::to_bytes(request.into_body(), 16 * 1024).await {
        Ok(body) => body.to_vec(),
        Err(error) => return crate::api::error_400(error.to_string()),
    };
    match forward(&state, &platform.resource_service, method, &path, body).await {
        Ok(response) => response,
        Err(error) => crate::api::error_500(format!("resource service: {error:#}")),
    }
}

pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    let result = async {
        let Some(platform) = state.lifecycle.platform.get() else {
            return Ok(serde_json::json!({"enabled":false}));
        };
        let bytes = tokio::fs::read(platform.state_dir.join("release-state.json")).await?;
        let release: serde_json::Value = serde_json::from_slice(&bytes)?;
        let response = forward(&state,&platform.host_service,reqwest::Method::GET,"/status",Vec::new()).await?;
        anyhow::ensure!(response.status().is_success(), "host status query failed");
        let host: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await?)?;
        Ok::<_,anyhow::Error>(serde_json::json!({"instance_release":platform.release_id,"release":release,"host":host,"signal_protocol":if cfg!(unix) { 1 } else { 0 },"retiring":state.lifecycle.retiring.load(std::sync::atomic::Ordering::SeqCst)}))
    }.await;
    match result {
        Ok(value) => crate::api::response(RpcReply::ok(value)),
        Err(error) => crate::api::error_500(error.to_string()),
    }
}
