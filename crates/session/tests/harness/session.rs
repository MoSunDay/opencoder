use opencoder_core::{harness::Harness, Config};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, Store};
use std::{path::Path, sync::Arc};

#[path = "binary.rs"]
mod binary;
use binary::fake_binary;

pub async fn session(root: &Path) -> (SessionState, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let cfg = Config::default();
    let mut session = SessionState::new(
        "wrap-test",
        opencoder_core::resolve_agent("act").unwrap(),
        cfg.clone(),
        opencoder_session::harness::configured_client(cfg),
        root.into(),
    )
    .with_store(store.clone());
    session.harness.harness = Harness::Codex;
    let bin = fake_binary(root);
    session
        .harness
        .envs
        .insert("PATH".into(), format!("{}:/usr/bin:/bin", bin.display()));
    session.harness.envs.insert(
        "CAPTURE".into(),
        root.join("capture.jsonl").display().to_string(),
    );
    (session, store)
}
