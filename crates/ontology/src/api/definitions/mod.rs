mod actions;
mod attributes;
mod entity_types;
mod relationship_types;
use crate::http::AppState;
use axum::Router;
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .merge(actions::routes())
        .merge(attributes::routes())
        .merge(entity_types::routes())
        .merge(relationship_types::routes())
}
