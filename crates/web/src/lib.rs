pub mod api;
pub mod api_control;
pub mod api_envs;
pub mod api_events;
pub mod api_inputs;
pub mod api_meta;
pub mod api_nodes;
pub mod api_nodes_ops;
pub mod api_ops;
pub mod api_questions;
pub mod api_subagents;
pub mod auth_sig_mw;
pub mod cmd;
pub mod control_state;
pub mod handle;
mod handle_lifecycle;
mod handle_questions;
pub mod html;
pub mod nodes_state;
pub mod sse_dedup;
pub mod sse_nodes;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use opencoder_store::{LibsqlStore, Store};

use crate::control_state::ControlHub;
use crate::handle::HandleMap;
use crate::nodes_state::NodeHub;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub workdir: std::path::PathBuf,
    pub handles: HandleMap,
    /// Broadcast hub for node-task sessions (they own no drain handle).
    pub nodes: Arc<NodeHub>,
    /// P3 message-relay control hub: per-node FIFO of control tasks plus the
    /// pending browser waiters they must wake. Payloads are never persisted.
    pub controls: Arc<ControlHub>,
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
        nodes: Arc::new(NodeHub::new()),
        controls: Arc::new(ControlHub::new()),
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
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Build the application router. `token = Some(t)` enables HMAC signature
/// auth on every route (production); `token = None` skips the middleware (used
/// by tests that build their own router with an injected `MockChatClient`).
pub fn build_app(state: Arc<AppState>, token: Option<String>, web: bool) -> axum::Router {
    let mut app = Router::<Arc<AppState>>::new();
    if web {
        app = app
            .route("/", get(html::index))
            .route("/static/:name", get(html::static_asset));
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
        // ── multi-node fleet (Phase 2) ────────────────────────────────
        .route("/api/nodes", get(api_nodes::list_nodes))
        .route("/api/nodes/register", post(api_nodes::post_register))
        .route("/api/nodes/tasks/claim", get(api_nodes_ops::claim))
        .route(
            "/api/nodes/tasks/:tid/events",
            get(sse_nodes::get_node_task_events).post(api_nodes_ops::post_events),
        )
        .route(
            "/api/nodes/tasks/:tid/status",
            post(api_nodes_ops::post_status),
        )
        .route("/api/nodes/tasks", get(api_nodes_ops::list_all_tasks))
        .route("/api/nodes/tasks/:tid", get(api_nodes_ops::get_task))
        .route(
            "/api/sessions/:id/task",
            get(api_nodes_ops::get_session_task),
        )
        .route("/api/nodes/:id/heartbeat", post(api_nodes::post_heartbeat))
        .route(
            "/api/nodes/:id/tasks",
            get(api_nodes::list_tasks).post(api_nodes::dispatch_task),
        )
        .route(
            "/api/nodes/:node_id/tasks/:tid/cancel",
            post(api_nodes_ops::cancel_task),
        )
        // ── P3 message relay (browser ⇄ server ⇄ worker) ─────────────
        .route(
            "/api/nodes/:id/messages",
            post(api_control::fetch_node_messages),
        )
        .route(
            "/api/nodes/:id/control_result",
            post(api_control::post_control_result),
        )
        .route("/api/nodes/:id/dialogs", get(api_control::list_dialogs))
        .route("/api/nodes/:id", delete(api_nodes::delete_node))
        .route("/api/health", get(api::health))
        // Unsigned clock bootstrap for signature clients (SPA).
        .route("/api/time", get(auth_sig_mw::server_time))
        .with_state(state);
    if let Some(t) = token {
        let sig = std::sync::Arc::new(auth_sig_mw::SigState::new(t));
        app = app.layer(axum::middleware::from_fn_with_state(
            Some(sig),
            auth_sig_mw::require_sig,
        ));
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
            nodes: Arc::new(crate::nodes_state::NodeHub::new()),
            controls: Arc::new(crate::control_state::ControlHub::new()),
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
            nodes: Arc::new(crate::nodes_state::NodeHub::new()),
            controls: Arc::new(crate::control_state::ControlHub::new()),
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
