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
pub fn error_400(msg: impl Into<String>) -> Response {
    super::error_400(msg.into())
}
pub fn error_404(msg: impl Into<String>) -> Response {
    super::error_404(&msg.into())
}
pub fn error_500(msg: impl Into<String>) -> Response {
    super::error_500(msg.into())
}
pub fn to_json<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("project record serialization")
}
pub fn rec_list<T: serde::Serialize>(records: impl IntoIterator<Item = T>) -> Vec<Value> {
    records.into_iter().map(to_json).collect()
}
