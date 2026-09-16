//! Retired playbook scheduling endpoints. Historical records remain queryable.
use axum::response::Response;
pub async fn dispatch() -> Response {
    super::brain::migration()
}
pub async fn trigger_scan() -> Response {
    super::brain::migration()
}
