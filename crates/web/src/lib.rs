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

pub fn data_dir_for(workdir: &std::path::Path) -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".opencoder"))
        .join("opencoder")
        .join(hash_of(workdir))
}

fn hash_of(p: &std::path::Path) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for byte in p.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(FNV_PRIME);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::hash_of;
    use std::path::Path;

    #[test]
    fn hash_of_is_stable_and_pinned() {
        assert_eq!(hash_of(Path::new("/tmp/opencoder-pin")), "ecd58ecfd9089443");
    }

    #[test]
    fn hash_of_distinguishes_paths() {
        assert_ne!(hash_of(Path::new("/a/b")), hash_of(Path::new("/a/bb")));
    }
}
