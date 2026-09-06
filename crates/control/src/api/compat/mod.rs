use crate::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
pub mod sessions;
pub mod workflows;
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/models", get(sessions::models))
        .route("/api/skills", get(sessions::skills))
        .route("/api/nodes/:id/tasks", post(sessions::task))
        .route("/api/nodes/:id/dialogs", get(sessions::dialogs))
        .route("/api/nodes/:node/tasks/:id/cancel", post(sessions::cancel))
        .route("/api/nodes/tasks/:id/events", get(super::stream::events))
        .route("/api/sessions/:id/task", get(sessions::owner))
        .route(
            "/api/dag/defs/:id",
            get(workflows::dag_definition).delete(workflows::delete_dag),
        )
        .route("/api/dag/defs/:id/dispatch", post(workflows::dispatch_dag))
        .route("/api/dag/runs", get(workflows::dags))
        .route("/api/dag/runs/:id", get(workflows::dag))
        .route("/api/dag/runs/:id/events", get(super::stream::events))
        .route("/api/dag/runs/:id/cancel", post(workflows::cancel))
        .route(
            "/api/todo/templates/:name/:version/run",
            post(workflows::dispatch_todos),
        )
        .route("/api/todo/workflows", get(workflows::todos))
        .route("/api/todo/workflows/:id", get(workflows::todo))
        .route("/api/todo/workflows/:id/events", get(super::stream::events))
        .route(
            "/api/todo/workflows/:id/interrupt",
            post(workflows::interrupt),
        )
        .route("/api/todo/workflows/:id/resume", post(workflows::resume))
        .route("/api/project/todos/:id/runs", get(super::project::runs))
        .route("/api/project/runs/:id/cancel", post(workflows::cancel))
}
