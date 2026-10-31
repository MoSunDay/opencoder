//! Freshness run driver: fan executions out, poll fleet records and persist
//! the run transitions the data plane allows.
use super::planning::{
    apply, bound, concurrency, execution_id, execution_input, executor_kind, executor_target,
    failed_outcome, merge_progress, outcome_from_read, poll_from_index, settle, transition_allowed,
    ItemOutcome, PollOutcome, MAX_ERROR_CHARS,
};
use crate::{api::executions, AppState};
use futures::StreamExt;
use opencoder_core::fleet::{CreateExecution, RpcReply};
use opencoder_ontology::{AppError, FreshnessItemStatus, FreshnessOperation, FreshnessRun, FreshnessRunItem, FreshnessRunStatus};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Fleet poll cadence while dispatched executions are reporting.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Wall-clock budget for one run; items still dispatched fail on expiry.
const RUN_TIMEOUT: Duration = Duration::from_secs(3600);
/// Request-lock scope serializing one driver per run.
pub(super) const LOCK_SCOPE: &str = "ontology_freshness";

/// Read the on-demand result summary of a finished execution and map it to
/// an item outcome. Shared with the startup reconciler.
pub(super) async fn summary_outcome(state: &AppState, execution_id: &str) -> ItemOutcome {
    let reply: RpcReply = crate::api::executions::results::read(state, execution_id).await;
    outcome_from_read(reply.status, &reply.body)
}

/// Minimal entity snapshot for items whose entity vanished between run
/// creation and dispatch; prompts degrade to the stored name.
fn fallback_entity(item: &FreshnessRunItem) -> Value {
    json!({ "id": item.entity_id.to_string(), "name": item.entity_name })
}

/// Entities of the policy type keyed by id; missing reads degrade to the
/// per-item fallback rather than failing the run.
async fn entities_of_type(state: &AppState, env: &str, policy: &Value) -> HashMap<Uuid, Value> {
    let Some(type_id) = policy["entity_type_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
    else {
        tracing::warn!("Freshness policy has no usable entity type");
        return HashMap::new();
    };
    let read = state.ontology.as_ref().map(|ontology| {
        ontology.freshness(
            env.to_owned(),
            FreshnessOperation::EntitiesOfType { type_id },
        )
    });
    let read = match read {
        Some(read) => read.await,
        None => Ok(json!({ "items": [] })),
    };
    match read {
        Ok(value) => value["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entity| {
                Uuid::parse_str(entity["id"].as_str()?).ok().map(|id| (id, entity))
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "Freshness entity listing failed");
            HashMap::new()
        }
    }
}

/// Submit one execution per item. A 202 marks the item dispatched with its
/// deterministic execution id; anything else fails the item with the
/// bounded submit error.
async fn submit_one(
    state: &Arc<AppState>,
    policy: &Value,
    run_id: Uuid,
    entity: Value,
    mut item: FreshnessRunItem,
) -> FreshnessRunItem {
    let kind = executor_kind(policy);
    let id = execution_id(kind.prefix(), run_id, item.entity_id);
    let reply = executions::submit(
        state,
        CreateExecution {
            id: id.clone(),
            kind,
            target: executor_target(policy),
            input: execution_input(policy, &entity),
            node_id: None,
        },
    )
    .await;
    if reply.status == 202 {
        item.status = FreshnessItemStatus::Dispatched;
        item.execution_id = Some(id);
    } else {
        item.status = FreshnessItemStatus::Failed;
        item.error = Some(bound(
            &format!("submit {}: {}", reply.status, reply.body),
            MAX_ERROR_CHARS,
        ));
    }
    item
}

/// Fan out submissions concurrently and restore the run's item order.
async fn fan_out(
    state: &Arc<AppState>,
    policy: &Value,
    run_id: Uuid,
    items: &[FreshnessRunItem],
    entities: &HashMap<Uuid, Value>,
) -> Vec<FreshnessRunItem> {
    let submitted: Vec<FreshnessRunItem> = futures::stream::iter(items.iter().cloned())
        .map(|item| {
            let entity = entities
                .get(&item.entity_id)
                .cloned()
                .unwrap_or_else(|| fallback_entity(&item));
            submit_one(state, policy, run_id, entity, item)
        })
        .buffer_unordered(concurrency(policy))
        .collect()
        .await;
    items
        .iter()
        .map(|item| {
            submitted
                .iter()
                .find(|done| done.entity_id == item.entity_id)
                .cloned()
                .unwrap_or_else(|| item.clone())
        })
        .collect()
}

/// Read one run through the data plane.
pub(super) async fn fetch(state: &AppState, env: &str, id: Uuid) -> Option<FreshnessRun> {
    let read = state
        .ontology
        .as_ref()?
        .freshness(env.to_owned(), FreshnessOperation::RunGet(id))
        .await;
    match read {
        Ok(value) => serde_json::from_value(value["item"].clone())
            .map_err(|error| tracing::warn!(%error, "Freshness run record is undecodable"))
            .ok(),
        Err(error) => {
            tracing::warn!(%error, "Freshness run read failed");
            None
        }
    }
}

/// Persist `run` at `status`. The stored run is re-read first so saves
/// carry the stored revision; a conflict re-reads once and re-applies local
/// progress by entity id. Illegal transitions (the stored run moved on)
/// abort and leave recovery to the reconciler.
pub(super) async fn save(
    state: &AppState,
    env: &str,
    run: &FreshnessRun,
    status: FreshnessRunStatus,
) -> Option<FreshnessRun> {
    let mut stored = fetch(state, env, run.id).await?;
    for attempt in 0..2 {
        if !transition_allowed(stored.status, status) {
            tracing::warn!(
                run = %run.id, from = ?stored.status, to = ?status,
                "Freshness run transition rejected; leaving it active"
            );
            return None;
        }
        let mut candidate = stored.clone();
        merge_progress(&mut candidate.items, &run.items);
        candidate.status = status;
        let result = state
            .ontology
            .as_ref()?
            .freshness(
                env.to_owned(),
                FreshnessOperation::RunSave {
                    run: candidate,
                    expected_revision: stored.revision,
                },
            )
            .await;
        match result {
            Ok(value) => {
                return match serde_json::from_value(value["item"].clone()) {
                    Ok(saved) => Some(saved),
                    Err(error) => {
                        tracing::warn!(%error, "Freshness run save reply is undecodable");
                        None
                    }
                }
            }
            Err(AppError::Conflict(_)) if attempt == 0 => {
                stored = fetch(state, env, run.id).await?;
            }
            Err(error) => {
                tracing::warn!(%error, run = %run.id, "Freshness run save failed");
                return None;
            }
        }
    }
    tracing::warn!(run = %run.id, "Freshness run save conflicted twice");
    None
}

/// Resolve dispatched items against the fleet index; returns how many items
/// are still in flight.
async fn poll_pass(state: &AppState, items: &mut [FreshnessRunItem]) -> usize {
    let mut in_flight = 0;
    for item in items.iter_mut() {
        if item.status != FreshnessItemStatus::Dispatched {
            continue;
        }
        let Some(execution) = item.execution_id.clone() else {
            apply(item, failed_outcome("dispatched without an execution id"));
            continue;
        };
        match state.fleet.index(&execution).await {
            Ok(Some(index)) => match poll_from_index(index.status) {
                PollOutcome::Pending => in_flight += 1,
                PollOutcome::Done => apply(item, summary_outcome(state, &execution).await),
                PollOutcome::Failed(outcome) => apply(item, outcome),
            },
            // Report lag: keep polling; the run deadline is the backstop.
            Ok(None) => in_flight += 1,
            Err(error) => {
                tracing::warn!(%error, execution, "Freshness fleet index read failed");
                in_flight += 1;
            }
        }
    }
    in_flight
}

/// Fail items still dispatched when the run deadline expires.
fn timeout_remaining(items: &mut [FreshnessRunItem]) {
    for item in items {
        if item.status == FreshnessItemStatus::Dispatched {
            apply(item, failed_outcome("run timed out"));
        }
    }
}

/// Drive one run end to end: fan out executions, poll their fleet records
/// and persist exactly the queued→running and running→terminal saves the
/// data plane allows. Never panics; failures log and leave the run to the
/// startup reconciler.
pub(crate) async fn execute(state: Arc<AppState>, env: String, policy: Value, run: FreshnessRun) {
    let _guard = match state
        .fleet
        .request_lock(LOCK_SCOPE, &run.id.to_string())
        .await
    {
        Ok(guard) => guard,
        Err(error) => {
            tracing::warn!(%error, run = %run.id, "Freshness run lock unavailable");
            return;
        }
    };
    let mut run = run;
    let entities = entities_of_type(&state, &env, &policy).await;
    // Dispatch before the first save: the data plane forbids running→running
    // saves, so dispatch results ride the queued→running transition.
    let submitted = fan_out(&state, &policy, run.id, &run.items, &entities).await;
    merge_progress(&mut run.items, &submitted);
    let Some(saved) = save(&state, &env, &run, FreshnessRunStatus::Running).await else {
        return;
    };
    run = saved;
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        if state.lifecycle.retiring.load(Ordering::SeqCst) {
            // The next activation's reconciler owns the outstanding items.
            tracing::warn!(run = %run.id, "Server retiring; freshness run left active");
            return;
        }
        if poll_pass(&state, &mut run.items).await == 0 {
            break;
        }
        if Instant::now() >= deadline {
            timeout_remaining(&mut run.items);
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    let status = settle(&run.items);
    if save(&state, &env, &run, status).await.is_none() {
        tracing::warn!(run = %run.id, ?status, "Freshness run settlement failed");
    }
}
