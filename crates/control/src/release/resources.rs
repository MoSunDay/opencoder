//! Independent NFS owner. Business servers only proxy its management API.
use anyhow::{ensure, Result};
use opencoder_core::Config;
use std::path::PathBuf;

pub async fn serve(workdir: PathBuf, data: PathBuf, port: u16, token: String) -> Result<()> {
    let config = Config::load(&workdir)?;
    let state = crate::bootstrap::new_resource_state(workdir.clone(), data.clone()).await?;
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
            crate::nfs_exports::DAG_BINARY_EXPORT,
            opencoder_agents::NfsServerOpts {
                export_root: config
                    .dag
                    .binary_dir
                    .clone()
                    .unwrap_or_else(|| opencoder_core::data_dir_for(&workdir).join("dag/binary")),
                host: config.dag.nfs.host.clone(),
                port: config.dag.nfs.port,
                read_only: true,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
    }
    if config.dag.workspace_nfs.enabled {
        crate::api_dag_workspace_nfs::start(&config)
            .await
            .map_err(anyhow::Error::msg)?;
    }
    crate::ontology::autostart(&config, &workdir, &data).await?;
    let app = build_app(state, token);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn build_app(state: std::sync::Arc<crate::AppState>, token: String) -> axum::Router {
    let store = state.store.clone();
    axum::Router::new()
        .merge(crate::routes::binary_resources(state.clone()))
        .route(
            "/api/health",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"ok":true,"role":"resources",
                    "build":opencoder_core::version::build_info()}))
            }),
        )
        .route(
            "/api/ontology/nfs",
            axum::routing::get(crate::ontology::status).post(crate::ontology::set_status),
        )
        .route(
            "/api/agents/nfs",
            axum::routing::get(crate::api_agent_nfs::get_status)
                .post(crate::api_agent_nfs::post_set),
        )
        .route(
            "/api/dag/binaries/nfs",
            axum::routing::get(crate::api_dag_binaries_nfs::nfs_get)
                .post(crate::api_dag_binaries_nfs::nfs_post),
        )
        .route(
            "/api/dag/workspace/nfs",
            axum::routing::get(crate::api_dag_workspace_nfs::get_status)
                .post(crate::api_dag_workspace_nfs::set_status),
        )
        .with_state(state)
        .layer(axum::middleware::from_fn(crate::role_gate::require_role))
        .layer(axum::middleware::from_fn_with_state(
            Some(std::sync::Arc::new(crate::auth_mw::AuthState::new(
                token, store,
            ))),
            crate::auth_mw::require_bearer,
        ))
}
