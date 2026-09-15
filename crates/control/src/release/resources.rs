//! Independent NFS owner. Business servers only proxy its management API.
use anyhow::{ensure, Result};
use opencoder_core::Config;
use std::path::PathBuf;

pub async fn serve(workdir: PathBuf, data: PathBuf, port: u16, token: String) -> Result<()> {
    let config = Config::load(&workdir)?;
    ensure!(
        config.agent.nfs.read_only && config.dag.nfs.read_only,
        "resource service requires read-only exports"
    );
    if config.agent.nfs.enabled {
        crate::api_agent_nfs::start_locked(&config)
            .await
            .map_err(anyhow::Error::msg)?;
    }
    if config.dag.nfs.enabled {
        crate::nfs_exports::start(
            crate::nfs_exports::DAG_WASM_EXPORT,
            opencoder_agents::NfsServerOpts {
                export_root: config
                    .dag
                    .wasm_dir
                    .clone()
                    .unwrap_or_else(|| opencoder_core::data_dir_for(&workdir).join("dag/wasm")),
                host: config.dag.nfs.host,
                port: config.dag.nfs.port,
                read_only: true,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    }
    let state = crate::new_state(workdir, data, None).await?;
    let store = state.store.clone();
    let app = axum::Router::new()
        .route(
            "/api/health",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"ok":true,"role":"resources"}))
            }),
        )
        .route(
            "/api/agents/nfs",
            axum::routing::get(crate::api_agent_nfs::get_status)
                .post(crate::api_agent_nfs::post_set),
        )
        .route(
            "/api/dag/wasm/nfs",
            axum::routing::get(crate::api_dag_wasm_nfs::nfs_get)
                .post(crate::api_dag_wasm_nfs::nfs_post),
        )
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            Some(std::sync::Arc::new(crate::auth_mw::AuthState::new(
                token, store,
            ))),
            crate::auth_mw::require_bearer,
        ));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
