use axum::extract::State;
use axum::response::Html;
use std::sync::Arc;

use crate::AppState;

pub async fn index(State(_state): State<Arc<AppState>>) -> Html<&'static str> {
    Html(MANAGER_HTML)
}

const MANAGER_HTML: &str = include_str!("manager.html");
