pub mod api;
pub mod api_ops;
pub mod auth;
pub mod cmd;
pub mod handle;
pub mod html;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, patch, post};
use axum::Router;
use opencoder_store::{LibsqlStore, Store};

use crate::handle::HandleMap;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub workdir: std::path::PathBuf,
    pub handles: HandleMap,
    pub client_override: Option<Arc<dyn opencoder_llm::ChatStream>>,
}

pub async fn serve(
    host: String,
    port: u16,
    _web: bool,
    workdir: std::path::PathBuf,
    token: String,
) -> Result<()> {
    let data_dir = data_dir_for(&workdir);
    tokio::fs::create_dir_all(&data_dir).await.ok();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(data_dir.join("opencoder.db")).await?);

    let state = Arc::new(AppState {
        store,
        workdir: workdir.clone(),
        handles: handle::new_handle_map(),
        client_override: None,
    });

    let app = build_app(state, Some(token));

    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!("opencoder {} listening on http://{addr}", opencoder_core::version::VERSION_LONG);
    println!("opencoder {} listening on http://{addr}", opencoder_core::version::VERSION_LONG);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Build the application router. `token = Some(t)` enables bearer-token auth on
/// every route (production); `token = None` skips the middleware (used by tests
/// that build their own router with an injected `MockChatClient`).
pub fn build_app(state: Arc<AppState>, token: Option<String>) -> axum::Router {
    let mut app = Router::new()
        .route("/", get(html::index))
        .route(
            "/api/sessions",
            get(api::list_sessions).post(api::create_session),
        )
        .route(
            "/api/sessions/:id",
            get(api::get_session).delete(api::delete_session),
        )
        .route("/api/sessions/:id/messages", get(api::get_messages))
        .route("/api/sessions/:id/prompt", post(api::post_prompt))
        .route("/api/sessions/:id/events", get(api::get_events))
        .route("/api/sessions/:id/seq", get(api::get_event_seq))
        .route("/api/sessions/:id/agent", post(api::post_agent))
        .route("/api/sessions/:id/model", post(api::post_model))
        .route("/api/sessions/:id/interrupt", post(api::post_interrupt))
        .route(
            "/api/sessions/:id/subagents/:task_id/steer",
            post(api::post_subagent_steer),
        )
        .route("/api/sessions/:id/fork", post(api_ops::fork_session))
        .route("/api/sessions/:id/compact", post(api_ops::post_compact))
        .route("/api/sessions/:id/handoff", post(api_ops::post_handoff))
        .route("/api/sessions/:id/skill", post(api_ops::post_skill))
        .route("/api/config", get(api_ops::get_config))
        .route("/api/config", patch(api_ops::patch_config))
        .route("/api/bg", get(api_ops::list_bg))
        .route("/api/bg/stop", post(api_ops::stop_bg))
        .route("/api/health", get(api::health))
        .with_state(state);
    if let Some(t) = token {
        app = app.layer(axum::middleware::from_fn_with_state(t, auth::require_token));
    }
    app
}

pub use opencoder_core::data_dir_for;

#[cfg(test)]
mod tests {
    use super::data_dir_for;
    use std::path::Path;

    #[test]
    fn data_dir_for_is_deterministic() {
        assert_eq!(
            data_dir_for(Path::new("/tmp/opencoder-pin")),
            data_dir_for(Path::new("/tmp/opencoder-pin"))
        );
    }

    #[test]
    fn data_dir_for_distinguishes_paths() {
        assert_ne!(
            data_dir_for(Path::new("/a/b")),
            data_dir_for(Path::new("/a/bb"))
        );
    }
}
