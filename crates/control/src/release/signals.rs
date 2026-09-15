//! Signals request an independent deployment job; they never stop this server.
use crate::AppState;
use anyhow::{ensure, Context, Result};
use std::{sync::Arc, time::Duration};

pub struct SignalTask(Option<tokio::task::JoinHandle<()>>);

impl Drop for SignalTask {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

pub async fn request(state: &AppState, action: &str) -> Result<serde_json::Value> {
    ensure!(
        matches!(action, "deploy" | "rollback"),
        "unknown release action"
    );
    ensure!(
        !state
            .lifecycle
            .retiring
            .load(std::sync::atomic::Ordering::SeqCst),
        "this server is retiring"
    );
    let platform = state
        .lifecycle
        .platform
        .get()
        .context("versioned deployment is not configured")?;
    let url = reqwest::Url::parse(&platform.host_service)?;
    ensure!(
        url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
        "Host must be loopback HTTP"
    );
    let token = state
        .lifecycle
        .credential
        .get()
        .context("service credential unavailable")?;
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()?
        .post(format!(
            "{}/deployment-signal",
            platform.host_service.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({"action":action,"release_id":platform.release_id}))
        .send()
        .await?;
    let status = response.status();
    let body: serde_json::Value = response.json().await?;
    ensure!(
        status.is_success(),
        "Host rejected {action}: {status}: {body}"
    );
    Ok(body)
}

pub fn start(state: Arc<AppState>) -> Result<SignalTask> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        // Install before the listener is advertised so neither signal can use
        // its default process-termination disposition on a ready server.
        let mut deploy = signal(SignalKind::user_defined2())?;
        let mut rollback = signal(SignalKind::user_defined1())?;
        Ok(SignalTask(Some(tokio::spawn(async move {
            loop {
                let action = tokio::select! {
                    value = deploy.recv() => { if value.is_none() { break; } "deploy" },
                    value = rollback.recv() => { if value.is_none() { break; } "rollback" },
                    _ = state.lifecycle.retired() => break,
                };
                match request(&state, action).await {
                    Ok(receipt) => tracing::info!(action, %receipt, "release signal accepted"),
                    Err(error) => {
                        tracing::error!(action, %error, "release signal failed; server remains serving")
                    }
                }
            }
        }))))
    }
    #[cfg(not(unix))]
    {
        let _ = state;
        Ok(SignalTask(None))
    }
}
