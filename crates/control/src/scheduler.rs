//! Control-plane cron scheduler: fires stored schedule jobs through the
//! existing execution entries (`executions::submit` for agent/team/todos/dag,
//! `brain_runs::create` for brain).
//!
//! The loop reuses the outbox pattern (`release::outbox::start`): an
//! `AtomicBool` lifecycle guard makes double-start a no-op, a weak handle
//! lets the server exit, and `lifecycle.retired()` breaks the sleep. Since
//! schema v27 the libsql `schedules` table is the definition source of
//! truth (CRUD via `POST/PUT/PATCH/DELETE /api/schedules`); the legacy
//! `schedules.json` is only a one-time bootstrap seed, and its
//! `scan_interval_secs` stays the ops knob (hot-read every loop). This loop
//! only submits executions and writes the `schedule_runs` ledger.
//!
//! Timing contract: every fire gets a deterministic id
//! `<kind>-<schedule_id>-<scheduled_for_ms>`, so re-firing a tick is
//! idempotent end to end. On catch-up only the most recent missed tick
//! fires — older ticks are recorded as `missed`. Creation, edits and re-enabling
//! start a new scheduling baseline; ticks before it are never backfilled.
//! An errored fire retries for up to one hour while its definition is unchanged.

use crate::{api, AppState};
use opencoder_core::{
    config::{ScheduleJob, ScheduleKind, ScheduleOverlap},
    fleet::*,
    message::now_ms,
    schedule::{parse_timezone, render_params, CronExpr},
};
use opencoder_store::{
    ScheduleDefRecord, ScheduleRunRecord, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED,
    SCHEDULE_RUN_MISSED,
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub mod telemetry;
mod timing;

/// Scan cadence when `schedules.json` does not pin `scan_interval_secs`.
const DEFAULT_SCAN_SECS: u64 = 15;

pub fn start(state: &Arc<AppState>) {
    if state
        .lifecycle
        .schedule_started
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut interval = DEFAULT_SCAN_SECS;
        let mut pending = HashMap::new();
        loop {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state
                .lifecycle
                .retiring
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            state.lifecycle.scheduler.scan_started(now_ms());
            if let Err(error) = scan(&state, &mut pending).await {
                state.lifecycle.scheduler.scan_error();
                tracing::error!(%error, "schedule scan failed");
            }
            state.lifecycle.scheduler.scan_completed(now_ms());
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                _ = state.lifecycle.retired() => return,
            }
            // Pick up `scan_interval_secs` edits without a restart.
            interval = opencoder_core::config::load_schedules(&state.workdir)
                .scan_interval_secs
                .unwrap_or(DEFAULT_SCAN_SECS)
                .max(1);
        }
    });
}

/// One scan pass: fire every enabled job's due tick. Definitions come from
/// the Store (`schedules` table — the same source the list surface shows);
/// per-job errors are logged and swallowed — a broken job must not starve
/// the others, and a store read failure only delays this scan.
async fn scan(
    state: &Arc<AppState>,
    pending: &mut HashMap<String, tokio::task::JoinHandle<()>>,
) -> Result<(), anyhow::Error> {
    pending.retain(|_, task| !task.is_finished());
    let defs = match state.store.list_schedules().await {
        Ok(defs) => defs,
        Err(error) => {
            state.lifecycle.scheduler.scan_error();
            tracing::error!(%error, "schedule scan read failed");
            return Ok(());
        }
    };
    for def in defs.iter().filter(|def| def.job.enabled) {
        let job = &def.job;
        if pending.contains_key(&job.id) {
            continue;
        }
        // Fail-soft: a structurally invalid job (bad cron / params / target
        // contract) is skipped with a warning; it must not starve the rest.
        if let Err(error) = job.validate() {
            state.lifecycle.scheduler.scan_error();
            tracing::warn!(schedule = %job.id, %error, "invalid schedule skipped");
            continue;
        }
        // A slow admission must not suspend scans of unrelated schedules.
        // Keep one in-flight fire per definition, including across scans;
        // durable outbox identity still governs retries after a restart.
        let state = state.clone();
        let def = def.clone();
        pending.insert(
            job.id.clone(),
            tokio::spawn(async move {
                if let Err(error) = fire_due(&state, &def).await {
                    state.lifecycle.scheduler.scan_error();
                    tracing::warn!(schedule = %def.job.id, %error, "schedule fire failed");
                }
            }),
        );
    }
    Ok(())
}

/// Compute the ticks due for `job` and fire the newest (older → `missed`).
async fn fire_due(state: &Arc<AppState>, def: &ScheduleDefRecord) -> anyhow::Result<()> {
    let job = &def.job;
    let expr = CronExpr::parse(&job.cron, job.timezone.as_deref())
        .map_err(|error| anyhow::anyhow!("cron: {error}"))?;
    let now = opencoder_core::message::now_ms();
    let last = state.store.last_schedule_run(&job.id).await?;

    // `overlap: skip` — the previous fire is still executing; wait for a
    // later tick instead of stacking runs. Unknown execution ids (index
    // rotated away) count as finished.
    if job.overlap == ScheduleOverlap::Skip {
        if let Some(last) = &last {
            if last.status == SCHEDULE_RUN_FIRED {
                if let Some(execution_id) = &last.execution_id {
                    if let Ok(Some(index)) = state.fleet.index(execution_id).await {
                        if !index.status.terminal() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    let Some(due) = timing::plan_due(&expr, def, last.as_ref(), now) else {
        return Ok(());
    };
    if let Some(missed) = due.missed_for_ms {
        record_missed(state, job, missed, now).await;
    }
    fire_tick(state, job, due.scheduled_for_ms, now).await?;
    Ok(())
}

/// Fire one tick: submit through the existing entry and persist the ledger
/// row (fired / error). A submission failure lands as an `error` row and
/// retries on later scans (see `fire_due`); the error also propagates so
/// the manual-fire endpoint (`POST /api/schedules/:id/run`) can surface it.
async fn fire_tick(
    state: &Arc<AppState>,
    job: &ScheduleJob,
    for_ms: i64,
    now: i64,
) -> anyhow::Result<String> {
    state.lifecycle.scheduler.fire_attempt();
    let execution_id = format!("{}-{}-{}", job.kind.as_str(), job.id, for_ms);
    let (status, failure) = match dispatch(state, job, &execution_id, for_ms).await {
        Ok(()) => (SCHEDULE_RUN_FIRED.to_string(), None),
        Err(error) => (SCHEDULE_RUN_ERROR.to_string(), Some(error.to_string())),
    };
    let rec = ScheduleRunRecord {
        schedule_id: job.id.clone(),
        kind: job.kind.as_str().to_string(),
        target: job.target.clone(),
        scheduled_for_ms: for_ms,
        fired_at_ms: now,
        execution_id: (status == SCHEDULE_RUN_FIRED).then(|| execution_id.clone()),
        status,
        error: failure.clone(),
        missed: false,
    };
    if let Err(error) = state.store.record_schedule_run(&rec).await {
        tracing::error!(schedule = %job.id, for_ms, %error, "record schedule run failed");
    }
    match failure {
        Some(error) => {
            state.lifecycle.scheduler.fire_error();
            Err(anyhow::anyhow!(error))
        }
        None => Ok(execution_id),
    }
}

/// Manual fire (`POST /api/schedules/:id/run`): submit one tick NOW and
/// record it at `scheduled_for_ms = now` (the deterministic id keeps it
/// idempotent with itself, and the ledger row doubles as the scan cursor).
/// Deliberately bypasses `enabled` and `overlap: skip` — it is an explicit
/// operator action, not a cron tick.
pub(crate) async fn fire_now(state: &Arc<AppState>, job: &ScheduleJob) -> anyhow::Result<String> {
    fire_tick(state, job, now_ms(), now_ms()).await
}

/// Record an older, skipped tick (`missed` rows carry no execution).
async fn record_missed(state: &Arc<AppState>, job: &ScheduleJob, for_ms: i64, now: i64) {
    state.lifecycle.scheduler.missed_tick();
    let rec = ScheduleRunRecord {
        schedule_id: job.id.clone(),
        kind: job.kind.as_str().to_string(),
        target: job.target.clone(),
        scheduled_for_ms: for_ms,
        fired_at_ms: now,
        execution_id: None,
        status: SCHEDULE_RUN_MISSED.to_string(),
        error: None,
        missed: true,
    };
    if let Err(error) = state.store.record_schedule_run(&rec).await {
        tracing::error!(schedule = %job.id, for_ms, %error, "record missed tick failed");
    }
}

async fn dispatch(
    state: &Arc<AppState>,
    job: &ScheduleJob,
    execution_id: &str,
    for_ms: i64,
) -> Result<(), anyhow::Error> {
    let offset = parse_timezone(job.timezone.as_deref().unwrap_or("utc"))
        .map_err(|error| anyhow::anyhow!("timezone: {error}"))?;
    let tick = chrono::DateTime::from_timestamp_millis(for_ms)
        .unwrap_or_else(chrono::Utc::now)
        .with_timezone(&offset);
    let params = render_params(&json_params(job), tick)
        .map_err(|error| anyhow::anyhow!("params: {error}"))?;
    let node_id = job.node_id.clone();
    match job.kind {
        ScheduleKind::Agent | ScheduleKind::Team | ScheduleKind::Todos | ScheduleKind::Dag => {
            let reply = api::executions::submit(
                state,
                CreateExecution {
                    id: execution_id.to_string(),
                    kind: job.kind.execution_kind(),
                    target: Some(job.target.clone()),
                    input: params,
                    node_id,
                },
            )
            .await;
            if reply.status == 202 {
                return Ok(());
            }
            anyhow::bail!("submit status {}: {}", reply.status, reply.body);
        }
        ScheduleKind::Brain => {
            let response = api::brain_runs::runs::create(
                axum::extract::State(state.clone()),
                axum::Json(brain_run(execution_id, node_id, &params)?),
            )
            .await;
            let status = response.status().as_u16();
            if status == 202 || status == 200 {
                return Ok(());
            }
            anyhow::bail!("brain run status {status}");
        }
    }
}

fn json_params(job: &ScheduleJob) -> Value {
    Value::Object(
        job.params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    )
}

/// Scheduled runs use exactly the same v3 admission contract as HTTP/CLI.
fn brain_run(execution_id: &str, node_id: Option<String>, params: &Value) -> anyhow::Result<Value> {
    anyhow::ensure!(
        params["schema_version"] == opencoder_core::brain::layered::LAYERED_SCHEMA_VERSION,
        "{}",
        opencoder_core::brain::layered::LAYERED_MIGRATION
    );
    let mut request = params.clone();
    request["id"] = serde_json::json!(execution_id);
    if let Some(node) = node_id {
        request["node_id"] = serde_json::json!(node);
    }
    Ok(request)
}
