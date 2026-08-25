pub mod api;
pub mod api_envs;
pub mod api_events;
pub mod api_inputs;
pub mod api_meta;
pub mod api_ops;
pub mod api_questions;
pub mod api_subagents;
pub mod auth;
pub mod cmd;
pub mod handle;
mod handle_lifecycle;
mod handle_questions;
pub mod html;
mod sse_dedup;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{delete, get, patch, post};
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
    web: bool,
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

    let app = build_app(state, Some(token), web);

    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!(
        "opencoder {} listening on http://{addr}",
        opencoder_core::version::VERSION_LONG
    );
    println!(
        "opencoder {} listening on http://{addr}",
        opencoder_core::version::VERSION_LONG
    );
    axum::serve(listener, app).await?;
    Ok(())
}

/// Build the application router. `token = Some(t)` enables bearer-token auth on
/// every route (production); `token = None` skips the middleware (used by tests
/// that build their own router with an injected `MockChatClient`).
pub fn build_app(state: Arc<AppState>, token: Option<String>, web: bool) -> axum::Router {
    let mut app = Router::<Arc<AppState>>::new();
    if web {
        app = app.route("/", get(html::index));
    }
    let mut app = app
        .route(
            "/api/sessions",
            get(api::list_sessions)
                .post(api::create_session)
                .delete(api_subagents::clear_sessions),
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
            "/api/sessions/:id/subagents",
            get(api_subagents::list_subagents),
        )
        .route(
            "/api/sessions/:id/subagents/:task_id/steer",
            post(api::post_subagent_steer),
        )
        .route("/api/sessions/:id/fork", post(api_ops::fork_session))
        .route("/api/sessions/:id/compact", post(api_ops::post_compact))
        .route("/api/sessions/:id/handoff", post(api_ops::post_handoff))
        .route("/api/sessions/:id/skill", post(api_ops::post_skill))
        .route(
            "/api/sessions/:id/questions",
            get(api_questions::list_questions),
        )
        .route(
            "/api/sessions/:id/questions/:call_id/answer",
            post(api_questions::answer_question),
        )
        .route(
            "/api/sessions/:id/questions/:call_id/skip",
            post(api_questions::skip_question),
        )
        .route("/api/sessions/:id/inputs", get(api_inputs::list_inputs))
        .route(
            "/api/sessions/:id/inputs/reorder",
            post(api_inputs::reorder_inputs),
        )
        .route(
            "/api/sessions/:id/inputs/:seq",
            delete(api_inputs::delete_input),
        )
        .route(
            "/api/sessions/:id/annotation",
            post(api_meta::post_annotation),
        )
        .route(
            "/api/sessions/:id/autopilot",
            post(api_meta::post_autopilot),
        )
        .route("/api/models", get(api_meta::get_models))
        .route("/api/skills", get(api_meta::get_skills))
        .route("/api/config", get(api_ops::get_config))
        .route("/api/config", patch(api_ops::patch_config))
        .route(
            "/api/envs",
            get(api_envs::list)
                .post(api_envs::create)
                .patch(api_envs::patch),
        )
        .route("/api/envs/:name/recapture", post(api_envs::recapture))
        .route("/api/envs/:name", delete(api_envs::delete))
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
    use super::{build_app, data_dir_for, handle, AppState};
    use std::path::Path;
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use opencoder_store::{LibsqlStore, Store};
    use tower::ServiceExt;

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

    /// Regression for `--web false`: the HTML UI route (`/`) must only be
    /// registered when `web == true`. Previously `serve` ignored its `web`
    /// argument (`_web: bool`) and `build_app` always wired `/`, so passing
    /// `--web false` still exposed the manager UI.
    async fn make_app() -> axum::Router {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let state = Arc::new(AppState {
            store,
            workdir: std::env::temp_dir(),
            handles: handle::new_handle_map(),
            client_override: None,
        });
        build_app(state, None, true)
    }

    async fn make_api_only_app() -> axum::Router {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let state = Arc::new(AppState {
            store,
            workdir: std::env::temp_dir(),
            handles: handle::new_handle_map(),
            client_override: None,
        });
        build_app(state, None, false)
    }

    #[tokio::test]
    async fn web_disabled_omits_html_route() {
        let app = make_api_only_app().await;
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "`/` must not be served when web is disabled"
        );
    }

    #[tokio::test]
    async fn web_enabled_serves_html_route() {
        let app = make_app().await;
        let resp = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "`/` must be served when web is enabled"
        );
    }
}
