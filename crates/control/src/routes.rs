use crate::{
    api::{self, admission, brain, catalog, executions, project, session, stream, streaming},
    *,
};
use axum::{
    routing::{get, patch, post, put},
    Router,
};
use serde_json::json;
use std::sync::Arc;

pub fn build_app(state: Arc<AppState>, token: Option<String>, web: bool) -> Router {
    let mut app = Router::<Arc<AppState>>::new()
        .merge(api::compat::routes())
        .route("/api/health", get(|| async { axum::Json(json!({"ok":true,"protocol_version":opencoder_core::fleet::PROTOCOL_VERSION,"role":"control","commit":opencoder_core::version::VERSION_LONG})) }))
        .route("/api/ready", get(admission::ready))
        .route(
            "/api/admin/drain",
            get(admission::status)
                .post(admission::freeze)
                .delete(admission::reopen),
        )
        .route("/api/time", get(auth_mw::server_time))
        .route("/api/nodes", get(catalog::nodes))
        .route("/api/nodes/channel", get(transport::upgrade))
        .route("/api/nodes/:id/maintenance", post(catalog::maintain))
        .route("/api/executions", get(executions::list).post(executions::create))
        .route("/api/executions/:id", get(executions::inspect))
        .route("/api/executions/:id/commands", post(executions::command))
        .route("/api/executions/:id/events", get(stream::events))
        .route(
            "/api/executions/:id/events/:seq/payload",
            get(executions::event_payload),
        )
        .route(
            "/api/executions/:id/detail-field",
            get(executions::detail_field),
        )
        .route("/api/executions/:id/messages", get(executions::messages))
        .route("/api/executions/:id/todo-items", get(executions::todo_items))
        .route("/api/executions/:id/project-runs", get(executions::project_runs))
        .route("/api/executions/:id/team-turns", get(executions::team_turns))
        .route("/api/executions/:id/artifact", get(streaming::artifact::download))
        .route("/api/sessions", get(session::list).post(session::create))
        .route("/api/sessions/:id/events", get(stream::events))
        .route("/api/teams", get(catalog::teams).post(catalog::save_team))
        .route("/api/dag/defs", get(catalog::dag_defs).post(catalog::save_dag))
        .route("/api/agents", get(api_agents::list).post(api_agents::create))
        .route("/api/agents/active", patch(api_agents::patch_active))
        .route("/api/agents/:name/meta", get(api_agents::meta))
        .route("/api/agents/:name", put(api_agents::update).delete(api_agents::delete))
        .route("/api/agents/resources/:cat", get(api_agent_resources::list).post(api_agent_resources::create))
        .route("/api/agents/resources/:cat/:name", put(api_agent_resources::put_version).delete(api_agent_resources::delete))
        .route("/api/agents/resources/:cat/:name/meta", get(api_agent_resources::meta))
        .route("/api/agents/resources/:cat/:name/rollback", post(api_agent_resources::rollback))
        .route("/api/agents/resources/:cat/:name/versions/:v/files/*path", get(api_agent_resources::read_file))
        .route("/api/agents/nfs", get(api_agent_nfs::get_status).post(api_agent_nfs::post_set))
        .route("/api/todo/envs", get(api_todo_envs::list_envs).post(api_todo_envs::create_env))
        .route("/api/todo/envs/:name", get(api_todo_envs::get_env).put(api_todo_envs::update_env).delete(api_todo_envs::delete_env))
        .route("/api/todo/tools", get(api_todo_envs::list_tools))
        .route("/api/todo/tools/import", post(api_todo_envs::import_tool))
        .route("/api/todo/templates", get(api_todo_templates::list_templates).post(api_todo_templates::create_template))
        .route("/api/todo/templates/:name", get(api_todo_templates::get_template).delete(api_todo_template_versions::delete_template))
        .route("/api/todo/templates/:name/todo.json", get(api_todo_templates::get_meta).put(api_todo_templates::update_meta))
        .route("/api/todo/templates/:name/new-version", post(api_todo_template_versions::new_version))
        .route("/api/todo/templates/:name/:version/context.json", get(api_todo_templates::get_context).put(api_todo_templates::put_context))
        .route("/api/todo/templates/:name/:version/env.json", get(api_todo_templates::get_env_binding).put(api_todo_templates::put_env_binding))
        .route("/api/todo/templates/:name/:version", axum::routing::delete(api_todo_template_versions::delete_version))
        .route("/api/project/overview", get(project::overview))
        .route("/api/project/goals", get(api_project::list_goals).post(api_project::create_goal))
        .route("/api/project/goals/:id", patch(api_project::patch_goal).delete(api_project::delete_goal))
        .route("/api/project/milestones", get(api_project::list_milestones).post(api_project::create_milestone))
        .route("/api/project/milestones/:id", patch(api_project::patch_milestone).delete(api_project::delete_milestone))
        .route("/api/project/todos", get(api_project_todos::list_todos).post(api_project_todos::create_todo))
        .route("/api/project/todos/:id", patch(api_project_todos::patch_todo).delete(api_project_todos::delete_todo))
        .route("/api/project/todos/:id/plan", post(project::plan))
        .route("/api/project/todos/:id/execute", post(project::execute))
        .route("/api/brain/capabilities", get(api_brain::list_capabilities).post(api_brain::create_capability))
        .route("/api/brain/capabilities/:id", get(api_brain::get_capability).put(api_brain::update_capability).delete(api_brain::delete_capability))
        .route("/api/brain/capabilities/:id/target", get(brain::target).put(brain::bind))
        .route("/api/brain/search", post(api_brain::search))
        .route("/api/brain/plans", post(brain::create_plan))
        .route("/api/brain/plans/:id", get(api_brain::get_plan))
        .route("/api/brain/preview", post(brain::preview))
        .route("/api/brain/dispatch", post(brain::dispatch))
        .fallback(api::session::relay);
    if web {
        app = app
            .route("/", get(html::index))
            .route("/static/:name", get(html::static_asset));
    }
    let mut app = app.with_state(state);
    if let Some(token) = token {
        app = app.layer(axum::middleware::from_fn_with_state(
            Some(Arc::new(auth_mw::AuthState::new(token))),
            auth_mw::require_bearer,
        ));
    }
    app
}
