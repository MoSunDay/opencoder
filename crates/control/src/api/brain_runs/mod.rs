mod catalog;
pub(crate) mod effects;
mod plans;
pub(crate) mod runs;
pub(crate) mod v3;
use crate::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/brain/plan-defs", get(plans::list).post(plans::save))
        .route("/api/brain/plan-defs/validate", post(plans::validate))
        .route("/api/brain/plan-defs/:id/versions", get(plans::versions))
        .route(
            "/api/brain/plan-defs/:id/versions/:version",
            get(plans::get),
        )
        .route("/api/brain/plan-defs/:id/stable", post(plans::stable))
        .route("/api/brain/plan-defs/:id/diff", get(plans::diff))
        .route("/api/brain/library", get(catalog::list))
        .route("/api/brain/library/:id/stable", post(catalog::stable))
        .route("/api/brain/runs", get(runs::list).post(runs::create))
        .route("/api/brain/runs/:id", get(runs::snapshot))
        .route("/api/brain/runs/:id/view", get(v3::view))
        .route("/api/brain/runs/:id/commands", post(runs::command))
        .route("/api/brain/runs/:id/inputs", post(runs::input))
        .route("/api/brain/runs/:id/context", get(runs::context))
        .route("/api/brain/runs/:id/actions", get(runs::actions))
        .route("/api/brain/runs/:id/instances", get(runs::instances))
        .route(
            "/api/brain/runs/:id/instances/:instance",
            get(runs::instance),
        )
        .route(
            "/api/brain/runs/:id/events",
            get(crate::api::stream::events),
        )
        .route("/api/brain/runs/:id/events-page", get(runs::events))
        .route("/api/brain/runs/:id/rounds/:round", get(v3::round))
}
