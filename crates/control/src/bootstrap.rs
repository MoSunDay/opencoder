use crate::{admission::AdmissionGate, transport::Hub, AppState};
use anyhow::{Context, Result};
use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatRequest, ChatStream, LlmEvent};
use opencoder_store::{fleet::FleetStore, LibsqlStore, Store};
use std::{path::PathBuf, sync::Arc, time::Duration};

const SERVER_SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

/// Resolves the two provider routes at the point of use. A server can manage
/// nodes without LLM credentials; a brain call reports the actual config error.
struct BrainClient {
    config: Config,
}
impl BrainClient {
    fn client(&self, embedding: bool) -> Result<ChatClient> {
        let cfg = &self.config;
        let ep = if embedding {
            cfg.resolve_embedding_endpoint()?
        } else {
            cfg.resolve_endpoint()?
        };
        ChatClient::new_with_read_timeout(
            &ep.base_url,
            &ep.api_key,
            &ep.headers,
            cfg.stream_idle_timeout(),
            cfg.network.proxy.as_deref(),
        )
    }
}
impl ChatStream for BrainClient {
    fn chat_stream(&self, req: ChatRequest) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        self.client(false)?
            .chat_stream(configured_planner_request(&self.config, req))
    }
    fn embed(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        self.client(true)?.embed(texts, model)
    }
    fn backend(&self) -> &'static str {
        "configured-brain"
    }
}

// Keep an explicit request override; otherwise inherit the server's existing
// reasoning setting, just like node conversations. Omitting it can spend the
// whole planner output budget on reasoning and leave an incomplete JSON tree.
fn configured_planner_request(config: &Config, mut request: ChatRequest) -> ChatRequest {
    if request.reasoning_effort.is_none() {
        request.reasoning_effort = config.reasoning_effort.clone();
    }
    request
}

pub async fn new_state(
    workdir: PathBuf,
    data: PathBuf,
    client: Option<Arc<dyn ChatStream>>,
) -> Result<Arc<AppState>> {
    tokio::fs::create_dir_all(&data).await?;
    let libsql = Arc::new(LibsqlStore::open(data.join("definitions.db")).await?);
    new_state_with_projects(workdir, data, client, libsql).await
}

/// Test/injection seam over `new_state`: swaps only the project store while
/// the shared `store` keeps the internal libsql. Production goes through
/// `new_state`.
pub async fn new_state_with_projects(
    workdir: PathBuf,
    data: PathBuf,
    client: Option<Arc<dyn ChatStream>>,
    projects: Arc<dyn opencoder_store::ProjectStore>,
) -> Result<Arc<AppState>> {
    tokio::fs::create_dir_all(&data).await?;
    let config = Config::load(&workdir)?;
    let libsql = Arc::new(LibsqlStore::open(data.join("definitions.db")).await?);
    let store: Arc<dyn Store> = libsql;
    let client = client.unwrap_or_else(|| {
        Arc::new(BrainClient {
            config: config.clone(),
        })
    });
    let brain = opencoder_brain::Runtime::new(store.clone(), client, config.embedding_model_id())
        .with_chat_model(config.small_model_or_primary());
    let fleet = Arc::new(FleetStore::open(&data.join("control.db")).await?);
    let admission = Arc::new(AdmissionGate::load(data.join("admission.json"))?);
    let hub = Arc::new(Hub::new(fleet.nodes().await?));
    for index in fleet.indexes(None, None, 10000).await? {
        if index.status == opencoder_core::fleet::ExecutionStatus::Pending {
            hub.reserve(&index).await;
        }
    }
    Ok(Arc::new(AppState {
        workdir,
        store,
        projects,
        fleet,
        hub,
        brain,
        brain_gate: Default::default(),
        admission,
        placement: tokio::sync::Mutex::new(()),
    }))
}

/// Record the startup token as the bootstrap admin user (idempotent). The
/// bearer middleware authenticates the seed token regardless of this table,
/// but the table row is what `GET /api/me` reports and what admins manage.
/// A name collision means the server restarted with a rotated startup
/// token: the `admin` row is re-pointed at the new digest so the previous
/// credential dies with the rotation instead of surviving it.
async fn seed_admin(store: &Arc<dyn Store>, token: &str) -> Result<()> {
    let digest = opencoder_core::identity::token_hash(token);
    if store.find_user_by_token_hash(&digest).await?.is_some() {
        return Ok(());
    }
    match store
        .create_user(
            "admin",
            &digest,
            opencoder_core::identity::Role::Admin,
            chrono::Utc::now().timestamp_millis(),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(create_error) => {
            // Only an admin-role `admin` row is a rotation target; a
            // deliberately created non-admin user named "admin" is left
            // untouched (warn and skip).
            let rotatable = match store.find_user_by_name("admin").await {
                Ok(Some(existing)) => Some(existing.role),
                _ => None,
            };
            if rotatable == Some(opencoder_core::identity::Role::Admin) {
                match store.update_user_token_hash("admin", &digest).await {
                    Ok(true) => {
                        tracing::info!("seed admin credential rotated to the new startup token");
                        return Ok(());
                    }
                    other => {
                        tracing::warn!(?other, "seed admin token rotation failed");
                    }
                }
            } else {
                tracing::warn!(%create_error, "seed admin user skipped (name already exists)");
            }
            Ok(())
        }
    }
}

pub async fn serve(
    host: String,
    port: u16,
    web: bool,
    workdir: PathBuf,
    data: Option<PathBuf>,
    token: String,
) -> Result<()> {
    let data = resolve_data_dir(&workdir, data)?;
    let state = new_state(workdir.clone(), data, None).await?;
    seed_admin(&state.store, &token).await?;
    let config = Config::load(&workdir)?;
    if config.agent.nfs.enabled {
        crate::api_agent_nfs::start_locked(&config)
            .await
            .map_err(anyhow::Error::msg)?;
    }
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    println!(
        "opencoder-server {} listening on http://{}",
        opencoder_core::version::VERSION_LONG,
        listener.local_addr()?
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server_state = Arc::clone(&state);
    let mut server = tokio::spawn(async move {
        axum::serve(
            listener,
            crate::build_app(server_state, Some(token), web)
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .await
    });
    tokio::select! {
        result = &mut server => {
            result.context("server task failed")??;
            return Ok(());
        }
        result = shutdown_signal(state) => result?,
    }
    let _ = stop.send(());
    match tokio::time::timeout(SERVER_SHUTDOWN_GRACE, &mut server).await {
        Ok(result) => {
            result.context("server task failed")??;
            Ok(())
        }
        Err(_) => {
            server.abort();
            let _ = server.await;
            anyhow::bail!("server connections did not drain within 30 seconds")
        }
    }
}

fn resolve_data_dir(workdir: &std::path::Path, explicit: Option<PathBuf>) -> Result<PathBuf> {
    let data = explicit.unwrap_or_else(|| opencoder_core::data_dir_for(workdir).join("server-v2"));
    anyhow::ensure!(data.is_absolute(), "server data directory must be absolute");
    Ok(data)
}

async fn shutdown_signal(state: Arc<AppState>) -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => {
                    tracing::error!(%error, "install SIGTERM handler");
                    let _ = tokio::signal::ctrl_c().await;
                    crate::api::admission::freeze_cluster(&state).await?;
                    state.hub.close_connections().await;
                    return Ok(());
                }
            };
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    tracing::error!(%error, "wait for Ctrl-C");
                }
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "wait for Ctrl-C");
    }
    crate::api::admission::freeze_cluster(&state).await?;
    state.hub.close_connections().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{configured_planner_request, resolve_data_dir};
    use opencoder_core::Config;
    use opencoder_llm::ChatRequest;
    use std::path::{Path, PathBuf};

    #[test]
    fn planner_inherits_configured_reasoning_and_preserves_request_overrides() {
        let config = Config {
            reasoning_effort: Some("low".into()),
            ..Config::default()
        };
        let request = ChatRequest {
            model: "planner".into(),
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            temperature: Some(0.2),
            max_tokens: Some(2048),
            reasoning_effort: None,
            cache_salt: None,
        };
        let inherited = configured_planner_request(&config, request.clone());
        assert_eq!(inherited.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(inherited.model, "planner");
        assert_eq!(inherited.max_tokens, Some(2048));
        let explicit = ChatRequest {
            reasoning_effort: Some("high".into()),
            ..request.clone()
        };
        assert_eq!(
            configured_planner_request(&config, explicit)
                .reasoning_effort
                .as_deref(),
            Some("high")
        );
        assert!(configured_planner_request(&Config::default(), request)
            .reasoning_effort
            .is_none());
    }

    #[test]
    fn explicit_data_dir_is_used_and_must_be_absolute() {
        let explicit = PathBuf::from("/srv/opencoder/server");
        assert_eq!(
            resolve_data_dir(Path::new("/work"), Some(explicit.clone())).unwrap(),
            explicit
        );
        assert!(resolve_data_dir(Path::new("/work"), Some(PathBuf::from("relative"))).is_err());
    }

    #[test]
    fn omitted_data_dir_keeps_the_existing_server_v2_default() {
        let workdir = Path::new("/tmp/opencoder-work");
        assert_eq!(
            resolve_data_dir(workdir, None).unwrap(),
            opencoder_core::data_dir_for(workdir).join("server-v2")
        );
    }
}

#[cfg(test)]
mod seed_admin_tests {
    use super::*;
    use opencoder_core::identity::{token_hash, Role};
    use opencoder_store::Store;

    async fn memory_store() -> Arc<dyn Store> {
        Arc::new(LibsqlStore::open_memory().await.unwrap())
    }

    async fn name_of_digest(store: &Arc<dyn Store>, token: &str) -> Option<String> {
        store
            .find_user_by_token_hash(&token_hash(token))
            .await
            .unwrap()
            .map(|user| user.name)
    }

    #[tokio::test]
    async fn first_boot_seeds_and_re_boots_are_idempotent() {
        let store = memory_store().await;
        seed_admin(&store, "boot-token").await.unwrap();
        assert_eq!(name_of_digest(&store, "boot-token").await, Some("admin".into()));
        seed_admin(&store, "boot-token").await.unwrap();
        assert_eq!(
            store.list_users().await.unwrap().len(),
            1,
            "same-token restart must not duplicate the admin row"
        );
    }

    #[tokio::test]
    async fn rotating_the_startup_token_repoints_the_admin_credential() {
        let store = memory_store().await;
        seed_admin(&store, "first-boot-token").await.unwrap();
        seed_admin(&store, "rotated-token").await.unwrap();
        // The old seed credential must die with the rotation; the new one
        // owns the row.
        assert_eq!(name_of_digest(&store, "first-boot-token").await, None);
        assert_eq!(name_of_digest(&store, "rotated-token").await, Some("admin".into()));
    }

    #[tokio::test]
    async fn rotation_never_touches_a_non_admin_row_named_admin() {
        let store = memory_store().await;
        store
            .create_user("admin", &token_hash("user-owned-token"), Role::User, 1)
            .await
            .unwrap();
        seed_admin(&store, "seed-token").await.unwrap();
        // The user-created row keeps its credential and role; the seed token
        // has no row of its own (the bearer middleware still authenticates it).
        let row = store.find_user_by_name("admin").await.unwrap().unwrap();
        assert_eq!(row.role, Role::User);
        assert_eq!(name_of_digest(&store, "seed-token").await, None);
        assert_eq!(
            name_of_digest(&store, "user-owned-token").await,
            Some("admin".into())
        );
    }
}
