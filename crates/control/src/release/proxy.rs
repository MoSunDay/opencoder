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
    bounded: bool,
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
    let mut client = reqwest::Client::builder().no_proxy();
    if bounded {
        client = client
            .connect_timeout(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(10));
    }
    let mut response = client
        .build()?
        .request(method, format!("{}{path}", base.trim_end_matches('/')))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await?;
    let limit = 48 * 1024 * 1024;
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= limit),
        "internal response is too large"
    );
    let mut builder = Response::builder().status(response.status().as_u16());
    for name in ["content-type", "content-disposition"] {
        if let Some(value) = response.headers().get(name) {
            builder = builder.header(name, value.clone());
        }
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= limit as usize,
            "internal response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(builder.body(axum::body::Body::from(bytes))?)
}

pub async fn forward_resources(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let binary = path == "/api/dag/binaries" || path.starts_with("/api/dag/binaries/");
    if !binary
        && !matches!(
            path,
            "/api/agents/nfs"
                | "/api/dag/workspace/nfs"
                | "/api/ontology/nfs"
                | "/api/project/todos/nfs"
        )
    {
        return next.run(request).await;
    }
    let todo = path == "/api/project/todos/nfs";
    let base = if todo {
        match opencoder_core::Config::load(&state.workdir) {
            Ok(config) => match config.project.todo_storage.nfs_service_url {
                Some(url) => url,
                None if state.lifecycle.platform.get().is_none() => return next.run(request).await,
                None => return unavailable("TODO resource service is not configured"),
            },
            Err(error) => return unavailable(&error.to_string()),
        }
    } else {
        let Some(platform) = state.lifecycle.platform.get() else {
            return next.run(request).await;
        };
        platform.resource_service.clone()
    };
    let path = request
        .uri()
        .path_and_query()
        .map(|path| path.as_str())
        .unwrap_or(path)
        .to_owned();
    let method = request.method().clone();
    let body = match axum::body::to_bytes(
        request.into_body(),
        if binary { 48 * 1024 * 1024 } else { 16 * 1024 },
    )
    .await
    {
        Ok(body) => body.to_vec(),
        Err(error) => return crate::api::error_400(error.to_string()),
    };
    match forward(&state, &base, method, &path, body, todo).await {
        Ok(response) => response,
        Err(error) if todo => unavailable(&format!("TODO resource service: {error:#}")),
        Err(error) => crate::api::error_500(format!("resource service: {error:#}")),
    }
}

fn unavailable(message: &str) -> Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error":message})),
    )
        .into_response()
}

pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    let result = async {
        let Some(platform) = state.lifecycle.platform.get() else {
            return Ok(serde_json::json!({"enabled":false}));
        };
        let bytes = tokio::fs::read(platform.state_dir.join("release-state.json")).await?;
        let release: serde_json::Value = serde_json::from_slice(&bytes)?;
        let response = forward(&state,&platform.host_service,reqwest::Method::GET,"/status",Vec::new(),false).await?;
        anyhow::ensure!(response.status().is_success(), "host status query failed");
        let host: serde_json::Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await?)?;
        Ok::<_,anyhow::Error>(serde_json::json!({"instance_release":platform.release_id,"release":release,"host":host,"signal_protocol":if cfg!(unix) { 1 } else { 0 },"retirement_protocol":if cfg!(target_os="linux") { 2 } else { 1 },"retiring":state.lifecycle.retiring.load(std::sync::atomic::Ordering::SeqCst)}))
    }.await;
    match result {
        Ok(value) => crate::api::response(RpcReply::ok(value)),
        Err(error) => crate::api::error_500(error.to_string()),
    }
}
