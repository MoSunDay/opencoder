pub mod api;
pub mod handle;
pub mod html;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use opencoder_store::{LibsqlStore, Store};

use crate::handle::HandleMap;

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub workdir: std::path::PathBuf,
    pub handles: HandleMap,
}

pub async fn serve(host: String, port: u16, _web: bool, workdir: std::path::PathBuf) -> Result<()> {
    let data_dir = data_dir_for(&workdir);
    tokio::fs::create_dir_all(&data_dir).await.ok();
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open(data_dir.join("opencoder.db")).await?);

    let state = Arc::new(AppState {
        store,
        workdir: workdir.clone(),
        handles: handle::new_handle_map(),
    });

    let app = Router::new()
        .route("/", get(html::index))
        .route(
            "/api/sessions",
            get(api::list_sessions).post(api::create_session),
        )
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
        .unwrap_or_else(|| std::path::PathBuf::from(".opencoder"))
        .join("opencoder")
        .join(hash_of(workdir))
}

/// Stable 64-bit fingerprint of a workdir path, used to key the local data
/// directory. Uses FNV-1a (deterministic, no extra dependency) rather than
/// `std::collections::hash_map::DefaultHasher`, which the std docs explicitly
/// warn is NOT stable across Rust versions — basing DB-path identity on it
/// would silently "lose" sessions after a toolchain bump. This is an identity
/// key, not a security primitive, so a non-cryptographic hash is appropriate.
fn hash_of(p: &std::path::Path) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for byte in p.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*byte);
        h = h.wrapping_mul(FNV_PRIME);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::hash_of;
    use std::path::Path;

    /// Pin the hash to a fixed value so any future change to the algorithm
    /// (which would silently remap workdirs to new data dirs and "lose"
    /// sessions) is caught at test time.
    #[test]
    fn hash_of_is_stable_and_pinned() {
        // FNV-1a 64 of the bytes of "/tmp/opencoder-pin"
        assert_eq!(hash_of(Path::new("/tmp/opencoder-pin")), "ecd58ecfd9089443");
    }

    #[test]
    fn hash_of_distinguishes_paths() {
        assert_ne!(hash_of(Path::new("/a/b")), hash_of(Path::new("/a/bb")));
    }
}
