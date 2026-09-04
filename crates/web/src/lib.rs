pub mod api;
pub mod api_agent_nfs;
pub mod api_agent_resources;
pub mod api_agents;
pub mod api_brain;
pub mod api_control;
pub mod api_dag;
pub mod api_envs;
pub mod api_events;
pub mod api_inputs;
pub mod api_meta;
pub mod api_nodes;
pub mod api_nodes_dag;
pub mod api_nodes_ops;
pub mod api_ops;
pub mod api_project;
pub mod api_project_runs;
pub mod api_project_todos;
pub mod api_project_util;
pub mod api_questions;
pub mod api_subagents;
pub mod api_teams;
pub mod api_teams_topics;
pub mod api_todo_envs;
pub mod api_todo_runs;
pub mod api_todo_template_versions;
pub mod api_todo_templates;
pub mod api_todo_util;
pub mod auth_sig_mw;
pub mod cmd;
pub mod control_state;
pub mod dag_state;
pub mod handle;
mod handle_lifecycle;
mod handle_questions;
pub mod html;
pub mod nodes_state;
pub mod sse_dag;
pub mod sse_dedup;
pub mod sse_nodes;
pub mod team_hub;
pub mod team_state;
pub mod todo_hub;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use opencoder_store::{LibsqlStore, Store};

use crate::control_state::ControlHub;
use crate::handle::HandleMap;
use crate::nodes_state::NodeHub;
use crate::team_state::TeamWebState;

pub use opencoder_project::ProjectService;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub workdir: std::path::PathBuf,
    pub handles: HandleMap,
    /// Broadcast hub for node-task sessions (they own no drain handle).
    pub nodes: Arc<NodeHub>,
    /// P3 message-relay control hub: per-node FIFO of control tasks plus the
    /// pending browser waiters they must wake. Payloads are never persisted.
    pub controls: Arc<ControlHub>,
    /// Project module runtime (goals → milestones → todos → plan/execute
    /// runs). `new()` is dependency-free so every AppState construction site
    /// stays cheap; `serve` (and integration tests) inject the real deps via
    /// `init` — until then every /api/project route answers 503.
    pub project: Arc<ProjectService>,
    /// Project brain (capability library) runtime: embedding-backed CRUD +
    /// semantic search over the same `store`. Wired to the primary provider
    /// endpoint in `serve`, degraded (bail-only client) when config fails.
    /// Multi-agent team-discussion runtime state: resolved team run config
    /// (team_root + turn bounds), the node dispatcher prompts fan out
    /// through, and the hub of live topic-runtime tasks.
    pub team: Arc<TeamWebState>,
    pub brain: opencoder_brain::Runtime,
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
    // Keep the concrete libsql arc: the project-data backend wants the SAME
    // store instance (one connection, one db_lock) when config says libsql.
    let libsql = Arc::new(LibsqlStore::open(data_dir.join("opencoder.db")).await?);
    let store: Arc<dyn Store> = libsql.clone();

    // Project module runtime. The project-data backend follows
    // config.storage (libsql default shares the same instance); optional
    // mysql/starrocks refuse cleanly when not compiled in — we log and fall
    // back to libsql rather than refusing to boot.
    let project = ProjectService::new();
    {
        let config = opencoder_core::Config::load(&workdir).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "config load failed; project storage falls back to libsql");
            opencoder_core::Config::default()
        });
        let projects = opencoder_store::open_project_store(&config.storage, libsql.clone())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "project storage backend unavailable; falling back to libsql");
                libsql.clone()
            });
        project
            .init(store.clone(), projects, workdir.clone(), None)
            .await?;
    }

    // Brain (capability library) rides the primary provider's endpoint — the
    // exact Config::load → resolve_endpoint → ChatClient chain the
    // `DrainCmd::ReloadConfig` branch in `handle.rs` uses (proxy, headers and
    // read-timeout included). Any failure degrades to a bail-only client so
    // serve() still boots and brain routes answer a clear 502.
    let brain = match opencoder_core::Config::load(&workdir) {
        Ok(cfg) => match cfg.resolve_endpoint() {
            Ok(ep) => match opencoder_llm::ChatClient::new_with_read_timeout(
                &ep.base_url,
                &ep.api_key,
                &ep.headers,
                cfg.stream_idle_timeout(),
                cfg.network.proxy.as_deref(),
            ) {
                Ok(c) => opencoder_brain::Runtime::new(
                    store.clone(),
                    Arc::new(c) as Arc<dyn opencoder_llm::ChatStream>,
                    cfg.embedding_model_id(),
                )
                .with_chat_model(cfg.small_model_or_primary()),
                Err(e) => {
                    tracing::warn!("brain degraded, llm client unavailable: {e:#}");
                    api_brain::degraded_brain(store.clone())
                }
            },
            Err(e) => {
                tracing::warn!("brain degraded, endpoint resolve failed: {e:#}");
                api_brain::degraded_brain(store.clone())
            }
        },
        Err(e) => {
            tracing::warn!("brain degraded, config load failed: {e:#}");
            api_brain::degraded_brain(store.clone())
        }
    };

    // Team runtime deps: resolved run config (team_root beside this
    // workdir's DB unless explicitly configured) + the node dispatcher.
    let team = crate::team_state::production(store.clone(), &workdir);
    if let Err(error) = tokio::fs::create_dir_all(&team.run.team_root).await {
        tracing::warn!(error = %error, "team root creation failed; team routes may fail until it exists");
    }
    let state = Arc::new(AppState {
        store,
        workdir: workdir.clone(),
        handles: handle::new_handle_map(),
        nodes: Arc::new(NodeHub::new()),
        controls: Arc::new(ControlHub::new()),
        project,
        team,
        brain,
        client_override: None,
    });

    // Daemon autostart for the agents NFS export: bring it up (when
    // `agent.nfs.enabled`) before the HTTP listener binds, so the API is
    // live from the first request. Failures only log — never fatal.
    api_agent_nfs::autostart(&workdir).await;

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
        // ── custom agents：reference cards + active marker + shared pools ──
        .route(
            "/api/agents",
            get(api_agents::list).post(api_agents::create),
        )
        .route("/api/agents/active", patch(api_agents::patch_active))
        .route("/api/agents/:name/meta", get(api_agents::meta))
        .route(
            "/api/agents/:name",
            put(api_agents::update).delete(api_agents::delete),
        )
        .route(
            "/api/agents/resources/:cat",
            get(api_agent_resources::list).post(api_agent_resources::create),
        )
        .route(
            "/api/agents/resources/:cat/:name",
            put(api_agent_resources::put_version).delete(api_agent_resources::delete),
        )
        .route(
            "/api/agents/resources/:cat/:name/meta",
            get(api_agent_resources::meta),
        )
        .route(
            "/api/agents/resources/:cat/:name/rollback",
            post(api_agent_resources::rollback),
        )
        .route(
            "/api/agents/resources/:cat/:name/versions/:v/files/*path",
            get(api_agent_resources::read_file),
        )
        .route(
            "/api/agents/nfs",
            get(api_agent_nfs::get_status).post(api_agent_nfs::post_set),
        )
        // ── TODO 管理（share 树）：envs / tools / templates / workflows ──
        .route(
            "/api/todo/envs",
            get(api_todo_envs::list_envs).post(api_todo_envs::create_env),
        )
        .route(
            "/api/todo/envs/:name",
            get(api_todo_envs::get_env)
                .put(api_todo_envs::update_env)
                .delete(api_todo_envs::delete_env),
        )
        .route("/api/todo/tools", get(api_todo_envs::list_tools))
        .route("/api/todo/tools/import", post(api_todo_envs::import_tool))
        .route(
            "/api/todo/templates",
            get(api_todo_templates::list_templates).post(api_todo_templates::create_template),
        )
        .route(
            "/api/todo/templates/:name",
            get(api_todo_templates::get_template)
                .delete(api_todo_template_versions::delete_template),
        )
        .route(
            "/api/todo/templates/:name/todo.json",
            put(api_todo_templates::update_meta).get(api_todo_templates::get_meta),
        )
        .route(
            "/api/todo/templates/:name/new-version",
            post(api_todo_template_versions::new_version),
        )
        .route(
            "/api/todo/templates/:name/:version/context.json",
            get(api_todo_templates::get_context).put(api_todo_templates::put_context),
        )
        .route(
            "/api/todo/templates/:name/:version/env.json",
            get(api_todo_templates::get_env_binding).put(api_todo_templates::put_env_binding),
        )
        .route(
            "/api/todo/templates/:name/:version",
            delete(api_todo_template_versions::delete_version),
        )
        .route(
            "/api/todo/templates/:name/:version/run",
            post(api_todo_runs::run_template),
        )
        .route("/api/todo/workflows", get(api_todo_runs::list_workflows))
        .route("/api/todo/workflows/:id", get(api_todo_runs::get_workflow))
        .route(
            "/api/todo/workflows/:id/interrupt",
            post(api_todo_runs::interrupt_workflow),
        )
        .route(
            "/api/todo/workflows/:id/resume",
            post(api_todo_runs::resume_workflow),
        )
        .route(
            "/api/todo/workflows/:id/events",
            get(todo_hub::workflow_events),
        )
        .route("/api/bg", get(api_ops::list_bg))
        .route("/api/bg/stop", post(api_ops::stop_bg))
        // ── project 模块（goals → milestones → todos → plan/execute runs）──
        .route("/api/project/overview", get(api_project_runs::get_overview))
        .route(
            "/api/project/goals",
            get(api_project::list_goals).post(api_project::create_goal),
        )
        .route(
            "/api/project/goals/:id",
            patch(api_project::patch_goal).delete(api_project::delete_goal),
        )
        .route(
            "/api/project/milestones",
            get(api_project::list_milestones).post(api_project::create_milestone),
        )
        .route(
            "/api/project/milestones/:id",
            patch(api_project::patch_milestone).delete(api_project::delete_milestone),
        )
        .route(
            "/api/project/todos",
            get(api_project_todos::list_todos).post(api_project_todos::create_todo),
        )
        .route(
            "/api/project/todos/:id",
            patch(api_project_todos::patch_todo).delete(api_project_todos::delete_todo),
        )
        .route(
            "/api/project/todos/:id/plan",
            post(api_project_runs::start_plan),
        )
        .route(
            "/api/project/todos/:id/execute",
            post(api_project_runs::start_execute),
        )
        .route(
            "/api/project/todos/:id/runs",
            get(api_project_runs::list_todo_runs),
        )
        .route(
            "/api/project/runs/:rid/cancel",
            post(api_project_runs::cancel_run),
        )
        // ── project brain (capability library) ───────────────────────
        .route(
            "/api/brain/capabilities",
            get(api_brain::list_capabilities).post(api_brain::create_capability),
        )
        .route(
            "/api/brain/capabilities/:id",
            get(api_brain::get_capability)
                .put(api_brain::update_capability)
                .delete(api_brain::delete_capability),
        )
        .route("/api/brain/search", post(api_brain::search))
        .route("/api/brain/plans", post(api_brain::create_plan))
        .route("/api/brain/plans/:id", get(api_brain::get_plan))
        .route("/api/brain/dispatch", post(api_brain::dispatch))
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
        // ── node-side DAG workflows (server stores + forwards only) ────
        .route(
            "/api/dag/defs",
            get(api_dag::list_defs).post(api_dag::post_def),
        )
        .route(
            "/api/dag/defs/:id",
            get(api_dag::get_def).delete(api_dag::delete_def),
        )
        .route("/api/dag/defs/:id/dispatch", post(api_dag::dispatch))
        .route("/api/dag/runs", get(api_dag::list_runs))
        .route("/api/dag/runs/:id", get(api_dag::get_run))
        .route("/api/dag/runs/:id/cancel", post(api_dag::cancel_run))
        .route("/api/dag/runs/:id/events", get(sse_dag::get_dag_run_events))
        .route("/api/nodes/dag/claim", get(api_nodes_dag::claim))
        .route(
            "/api/nodes/dag/runs/:rid/events",
            post(api_nodes_dag::post_events),
        )
        .route(
            "/api/nodes/dag/runs/:rid/status",
            post(api_nodes_dag::post_status),
        )
        // ── team orchestration (opencoder-team) ──────────────────────
        .merge(api_teams::routes())
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
            brain: crate::api_brain::mock_brain(store.clone()),
            store,
            workdir: std::env::temp_dir(),
            handles: handle::new_handle_map(),
            nodes: Arc::new(crate::nodes_state::NodeHub::new()),
            controls: Arc::new(crate::control_state::ControlHub::new()),
            team: crate::team_state::mock(),
            project: crate::ProjectService::new(),
            client_override: None,
        });
        build_app(state, None, true)
    }

    async fn make_api_only_app() -> axum::Router {
        let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let state = Arc::new(AppState {
            brain: crate::api_brain::mock_brain(store.clone()),
            store,
            workdir: std::env::temp_dir(),
            handles: handle::new_handle_map(),
            nodes: Arc::new(crate::nodes_state::NodeHub::new()),
            controls: Arc::new(crate::control_state::ControlHub::new()),
            team: crate::team_state::mock(),
            project: crate::ProjectService::new(),
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
