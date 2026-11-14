use crate::AppState;
use axum::response::Response;
use opencoder_store::ProjectStore;
use serde_json::Value;
use std::sync::Arc;
pub struct Deps {
    pub projects: Arc<dyn ProjectStore>,
}
pub fn require_deps(state: &AppState) -> Result<Arc<Deps>, Box<Response>> {
    Ok(Arc::new(Deps {
        projects: state.projects.clone(),
    }))
}
// Re-export the single error-constructor implementations: the stateless
// `/api/project` handlers shared with the node API are mounted into this
// crate from `../../web/src` and reach them through
// `crate::api_project_util::error_*` (the `lib.rs` alias).
pub use super::{error_400, error_404, error_409, error_500};
pub fn to_json<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("project record serialization")
}
pub fn rec_list<T: serde::Serialize>(records: impl IntoIterator<Item = T>) -> Vec<Value> {
    records.into_iter().map(to_json).collect()
}
