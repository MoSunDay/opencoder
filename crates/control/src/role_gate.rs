//! Authorization before every native route and session relay.
use axum::{
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use opencoder_core::{
    fleet::ExecutionKind,
    identity::{Identity, Role},
};
use serde_json::json;
use std::sync::Arc;

pub fn allowed(role: Role, method: &Method, path: &str) -> bool {
    opencoder_core::identity::permissions::allowed(role, method.as_str(), path)
}

pub async fn require_role(
    State(state): State<Arc<crate::AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(identity) = req.extensions().get::<Identity>() else {
        return next.run(req).await;
    };
    let path = req.uri().path();
    if !allowed(identity.role, req.method(), path) {
        return denied();
    }
    if !identity.is_admin() {
        if let Some(rest) = path
            .strip_prefix("/api/executions/")
            .or_else(|| path.strip_prefix("/api/sessions/"))
        {
            let id = rest.split('/').next().unwrap_or_default();
            if id.contains('%') {
                return denied();
            }
            match state.fleet.index(id).await {
                Ok(Some(index))
                    if matches!(
                        index.kind,
                        ExecutionKind::Maintenance | ExecutionKind::System
                    ) =>
                {
                    return denied()
                }
                Err(_) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error":"execution authorization lookup failed"})),
                    )
                        .into_response()
                }
                _ => {}
            }
        }
    }
    next.run(req).await
}

fn denied() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"ok":false,"error":"当前角色无权执行此操作"})),
    )
        .into_response()
}
