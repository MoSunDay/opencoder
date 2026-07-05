pub mod api;
pub mod handle;
pub mod html;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use opencode_store::{LibsqlStore, Store};

use crate::handle::HandleMap;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub workdir: std::path::PathBuf,
    pub handles: HandleMap,
}

pub async fn serve(host: String, port: u16, _web: bool) -> Result<()> {
    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let data_dir = data_dir_for(&workdir);
    tokio::fs::create_dir_all(&data_dir).await.ok();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(data_dir.join("opencode.db")).await?);

    let state = Arc::new(AppState {
        store,
        workdir: workdir.clone(),
        handles: handle::new_handle_map(),
    });

    let app = Router::new()
        .route("/", get(html::index))
        .route("/api/sessions", get(api::list_sessions).post(api::create_session))
        .route("/api/sessions/:id", get(api::get_session))
        .route("/api/sessions/:id/messages", get(api::get_messages))
        .route("/api/sessions/:id/prompt", post(api::post_prompt))
        .route("/api/sessions/:id/events", get(api::get_events))
        .route("/api/sessions/:id/agent", post(api::post_agent))
        .route("/api/sessions/:id/model", post(api::post_model))
        .route("/api/sessions/:id/interrupt", post(api::post_interrupt))
        .route("/api/health", get(api::health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    let addr = listener.local_addr()?;
    tracing::info!("opencoder web/server listening on http://{addr}");
    println!("opencoder web/server listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn data_dir_for(workdir: &std::path::Path) -> std::path::PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".opencode"))
        .join("opencode")
        .join(hash_of(workdir))
}

fn hash_of(p: &std::path::Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    format!("{:016x}", h.finish())
}
