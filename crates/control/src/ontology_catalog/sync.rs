use crate::{release::Lifecycle, AppState};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use opencoder_ontology::{AppError, CatalogOperation};
use serde_json::Value;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

struct Active(Arc<Lifecycle>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.requests.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

pub(super) async fn synchronize(
    state: &Arc<AppState>,
    env: String,
    check: bool,
) -> Result<Value, AppError> {
    let active = {
        let _gate = state.lifecycle.request_gate.lock().unwrap();
        if state.lifecycle.retiring.load(Ordering::SeqCst) {
            return Err(AppError::dependency("Server is retiring"));
        }
        state.lifecycle.requests.fetch_add(1, Ordering::SeqCst);
        Active(state.lifecycle.clone())
    };
    let state = state.clone();
    // A disconnected explicit caller cannot cancel an accepted synchronization.
    tokio::spawn(async move {
        let _active = active;
        let ontology = state.ontology.as_ref().ok_or(AppError::NotFound)?;
        let _lock = state
            .fleet
            .request_lock("ontology_catalog", &env)
            .await
            .map_err(AppError::from)?;
        let _writer = if check {
            None
        } else {
            super::ownership::writer(&state.lifecycle).await?
        };
        if state.lifecycle.retiring.load(Ordering::SeqCst) {
            return Err(AppError::dependency("Server is retiring"));
        }
        if !check {
            ontology
                .catalog(env.clone(), CatalogOperation::Start)
                .await?;
        }
        let entries = match super::source::snapshot(&state).await {
            Ok(entries) => entries,
            Err(error) => {
                if !check {
                    ontology
                        .catalog(env.clone(), CatalogOperation::Error(error.to_string()))
                        .await?;
                }
                return Err(AppError::dependency(error.to_string()));
            }
        };
        let result = ontology
            .catalog(
                env.clone(),
                if check {
                    CatalogOperation::Check(entries)
                } else {
                    CatalogOperation::Apply(entries)
                },
            )
            .await;
        if !check {
            if let Err(error) = &result {
                let _ = ontology
                    .catalog(env, CatalogOperation::Error(error.to_string()))
                    .await;
            }
        }
        result
    })
    .await
    .map_err(|error| AppError::dependency(error.to_string()))?
}

pub(crate) fn start(state: &Arc<AppState>) {
    if state.ontology.is_none() || state.lifecycle.catalog_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.lifecycle.retiring.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = synchronize(&state, "debug".into(), false).await {
                tracing::warn!(%error, "Ontology capability synchronization failed");
            }
            // Keep only Lifecycle while waiting, so dropping AppState stops the task.
            let lifecycle = state.lifecycle.clone();
            drop(state);
            tokio::select! {
                _ = lifecycle.retired() => return,
                _ = lifecycle.catalog_changed.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(30)) => {},
            }
        }
    });
}

pub(crate) async fn notify_changes(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let changes_definition = !matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS")
        && matches!(
            opencoder_core::ontology::platform_for_path(request.uri().path()),
            Some("agents" | "dag" | "todo" | "teams" | "brain" | "harness")
        );
    let response = next.run(request).await;
    if changes_definition && response.status().is_success() {
        state.lifecycle.catalog_changed.notify_one();
    }
    response
}
