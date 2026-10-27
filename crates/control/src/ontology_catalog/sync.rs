use crate::{release::Lifecycle, AppState};
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use opencoder_core::ontology::Capability;
use opencoder_ontology::{AppError, CatalogOperation};
use serde_json::Value;
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};

struct Active(Arc<Lifecycle>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.requests.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

/// Timer cycles within this window may skip catalog writes when the freshly
/// built snapshot matches the last successful Apply; afterwards a full cycle
/// re-runs to reaffirm and heal out-of-band catalog edits.
const MEMO_TTL: Duration = Duration::from_secs(10 * 60);

/// In-process memory of the last successful Apply, used only by the timer.
#[derive(Default)]
struct Memo {
    entries: Vec<Capability>,
    applied_at: Option<Instant>,
}

impl Memo {
    fn skips(&self, entries: &[Capability], now: Instant) -> bool {
        self.applied_at.is_some_and(|applied_at| {
            now.duration_since(applied_at) < MEMO_TTL && self.entries == entries
        })
    }
}

/// A finished cycle: the caller-visible result plus the memo carried into the
/// next timer cycle. Any failure drops the memo so the next cycle retries the
/// full path.
struct Outcome {
    result: Result<Value, AppError>,
    memo: Option<Memo>,
}

pub(super) async fn synchronize(
    state: &Arc<AppState>,
    env: String,
    check: bool,
) -> Result<Value, AppError> {
    cycle(state, env, check, None).await.0
}

async fn cycle(
    state: &Arc<AppState>,
    env: String,
    check: bool,
    memo: Option<Memo>,
) -> (Result<Value, AppError>, Option<Memo>) {
    let active = {
        let _gate = state.lifecycle.request_gate.lock().unwrap();
        if state.lifecycle.retiring.load(Ordering::SeqCst) {
            return (Err(AppError::dependency("Server is retiring")), None);
        }
        state.lifecycle.requests.fetch_add(1, Ordering::SeqCst);
        Active(state.lifecycle.clone())
    };
    let state = state.clone();
    // A disconnected explicit caller cannot cancel an accepted synchronization.
    let joined = tokio::spawn(async move {
        let _active = active;
        let ontology = state.ontology.as_ref().ok_or(AppError::NotFound)?;
        let _lock = state
            .fleet
            .request_lock("ontology_catalog", &env)
            .await
            .map_err(AppError::from)?;
        // Timer cycles build the source snapshot before any catalog write: the
        // snapshot is the change detector and source reads stay serialized by
        // the request lock above. An unchanged Apply inside the memo TTL skips
        // the writer lease, Start and Apply entirely.
        let snapshot = match memo.as_ref() {
            Some(_) => Some(super::source::snapshot(&state).await),
            None => None,
        };
        let skip = matches!(
            (memo.as_ref(), snapshot.as_ref()),
            (Some(memo), Some(Ok(entries))) if memo.skips(entries, Instant::now())
        );
        if skip {
            return Ok(Outcome {
                result: Ok(Value::Null),
                memo,
            });
        }
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
        let snapshot = match snapshot {
            Some(result) => result,
            None => super::source::snapshot(&state).await,
        };
        let entries = match snapshot {
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
        let memo_entries = (!check && memo.is_some()).then(|| entries.clone());
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
        // Refresh the memo only after a successful Apply.
        let memo = match (&result, memo_entries) {
            (Ok(_), Some(entries)) => Some(Memo {
                entries,
                applied_at: Some(Instant::now()),
            }),
            _ => memo,
        };
        Ok(Outcome { result, memo })
    })
    .await;
    match joined {
        Ok(Ok(outcome)) => (outcome.result, outcome.memo),
        Ok(Err(error)) => (Err(error), None),
        Err(error) => (Err(AppError::dependency(error.to_string())), None),
    }
}

pub(crate) fn start(state: &Arc<AppState>) {
    if state.ontology.is_none() || state.lifecycle.catalog_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        // Only this timer path consults the memo; explicit API synchronizations
        // always run the full cycle.
        let mut memo: Option<Memo> = None;
        loop {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.lifecycle.retiring.load(Ordering::SeqCst) {
                return;
            }
            let (result, carried) = cycle(&state, "debug".into(), false, memo.take()).await;
            memo = carried;
            if let Err(error) = result {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(keys: &[&str]) -> Vec<Capability> {
        keys.iter()
            .map(|key| Capability {
                key: (*key).into(),
                ..Default::default()
            })
            .collect()
    }

    // The sync loop itself needs a full AppState, so the timer's no-op
    // decision is covered through this pure helper.
    #[test]
    fn memo_skips_only_recent_unchanged_entries() {
        let now = Instant::now();
        let memo = Memo {
            entries: entries(&["a", "b"]),
            applied_at: Some(now),
        };
        assert!(memo.skips(&entries(&["a", "b"]), now));
        assert!(memo.skips(
            &entries(&["a", "b"]),
            now + MEMO_TTL - Duration::from_secs(1)
        ));
        // At or beyond the TTL the full cycle re-runs to heal drift.
        assert!(!memo.skips(&entries(&["a", "b"]), now + MEMO_TTL));
        assert!(!memo.skips(&entries(&["a"]), now));
        assert!(!memo.skips(&entries(&["a", "b", "c"]), now));
        assert!(!memo.skips(&entries(&["b", "a"]), now));
        // Never applied (initial state) or cleared after a failure.
        assert!(!Memo::default().skips(&[], now));
    }
}
