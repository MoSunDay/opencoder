//! Control-plane cron scheduler: fires `schedules.json` jobs through the
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
//! fires — older ticks are recorded as `missed`. An errored fire retries
//! on later scans while inside [`RETRY_WINDOW_MS`], then gives up.

use crate::{api, AppState};
use opencoder_core::{
    config::{ScheduleJob, ScheduleKind, ScheduleOverlap},
    fleet::*,
    message::now_ms,
    schedule::{parse_timezone, render_params, to_utc, CronExpr},
};
use opencoder_store::{
    ScheduleRunRecord, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED, SCHEDULE_RUN_MISSED,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// How far back a tick can be and still fire: server downtime up to a day
/// still catches up on the last cron tick; anything older is `missed`.
const CATCHUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// An errored fire retries on later scans inside this window, then waits
/// for the next cron tick instead of spinning.
const RETRY_WINDOW_MS: i64 = 60 * 60 * 1000;
/// Hard cap on candidate ticks per scan: the 24h walk-back of the densest
/// sane cron (every minute) still fits, denser 6-field crons collapse.
const MAX_CANDIDATE_TICKS: usize = 2_000;
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
            if let Err(error) = scan(&state).await {
                tracing::error!(%error, "schedule scan failed");
            }
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
async fn scan(state: &Arc<AppState>) -> Result<(), anyhow::Error> {
    let defs = match state.store.list_schedules().await {
        Ok(defs) => defs,
        Err(error) => {
            tracing::error!(%error, "schedule scan read failed");
            return Ok(());
        }
    };
    for def in defs.iter().filter(|def| def.job.enabled) {
        let job = &def.job;
        // Fail-soft: a structurally invalid job (bad cron / params / target
        // contract) is skipped with a warning; it must not starve the rest.
        if let Err(error) = job.validate() {
            tracing::warn!(schedule = %job.id, %error, "invalid schedule skipped");
            continue;
        }
        if let Err(error) = fire_due(state, job).await {
            tracing::warn!(schedule = %job.id, %error, "schedule fire failed");
        }
    }
    Ok(())
}

/// Compute the ticks due for `job` and fire the newest (older → `missed`).
async fn fire_due(state: &Arc<AppState>, job: &ScheduleJob) -> anyhow::Result<()> {
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

    // An errored fire retries in place while inside the retry window.
    if let Some(last) = &last {
        if last.status == SCHEDULE_RUN_ERROR
            && now.saturating_sub(last.scheduled_for_ms) < RETRY_WINDOW_MS
        {
            fire_tick(state, job, last.scheduled_for_ms, now).await?;
            return Ok(());
        }
    }

    // Due ticks: strictly after the last recorded tick, inside the catch-up
    // window, not in the future. The newest one fires; older ticks collapse
    // into a single `missed` representative row (their fire time is the
    // walk-back start), so a very dense cron cannot flood the ledger.
    let after = last
        .as_ref()
        .map(|run| run.scheduled_for_ms)
        .unwrap_or(0)
        .max(now - CATCHUP_WINDOW_MS);
    let Some(newest) = due_ticks(&expr, after, now).pop() else {
        return Ok(());
    };
    let first = expr.next_after(to_utc(after));
    match first {
        // More than one tick was due: audit the skipped window with its
        // oldest tick, then fire the newest.
        Some(first) if first.timestamp_millis() < newest => {
            record_missed(state, job, first.timestamp_millis(), now).await
        }
        _ => {}
    }
    fire_tick(state, job, newest, now).await?;
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
        Some(error) => Err(anyhow::anyhow!(error)),
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

/// Newest due tick for the walk-back window, or `None`. Iterates the cron
/// forward from `after_ms`; when the window is denser than
/// [`MAX_CANDIDATE_TICKS`] the scan narrows toward `now` so the newest tick
/// is still found (dense-cron catch-up history is collapsed anyway).
fn due_ticks(expr: &CronExpr, after: i64, now: i64) -> Vec<i64> {
    let mut span = CATCHUP_WINDOW_MS;
    loop {
        let start = after.max(now - span);
        let upcoming = expr.upcoming(to_utc(start), MAX_CANDIDATE_TICKS + 1);
        let due: Vec<i64> = upcoming
            .iter()
            .map(|tick| tick.timestamp_millis())
            .take_while(|ms| *ms <= now)
            .collect();
        if due.len() <= MAX_CANDIDATE_TICKS || start >= now {
            return due;
        }
        // Truncated: a dense cron fills the scan. Zoom into the recent past.
        span /= 4;
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
        params["schema_version"] == 3,
        "{}",
        opencoder_core::brain::SCHEDULER_MIGRATION
    );
    let mut request = params.clone();
    request["id"] = serde_json::json!(execution_id);
    if let Some(node) = node_id {
        request["node_id"] = serde_json::json!(node);
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed epoch: 2027-01-15T08:00:00Z, safely in the future.
    const NOW_MS: i64 = 1_800_000_000_000;

    #[test]
    fn per_second_cron_zooms_to_the_newest_ticks() {
        let expr = CronExpr::parse("* * * * * *", None).unwrap();
        let now = NOW_MS;
        let after = now - CATCHUP_WINDOW_MS;
        let due = due_ticks(&expr, after, now);
        assert!(
            !due.is_empty(),
            "a due tick must be found in a dense window"
        );
        assert!(due.len() <= MAX_CANDIDATE_TICKS, "candidate cap must hold");
        // Ascending, within the (clipped) walk-back window, none in the future.
        assert!(due.windows(2).all(|w| w[0] < w[1]));
        assert!(due.iter().all(|ms| *ms <= now && *ms > after));
        // The zoom must land on recent ticks: the newest second boundary at or
        // before `now` — not the first 2000 seconds of the 24h walk-back.
        let newest = now - now.rem_euclid(1000);
        assert_eq!(*due.last().unwrap(), newest);
        assert!(*due.last().unwrap() > now - 1_400_000);
    }

    #[test]
    fn daily_cron_walks_back_within_the_catchup_window() {
        let expr = CronExpr::parse("0 5 * * *", None).unwrap();
        let now = NOW_MS; // 2027-01-15T08:00:00Z
                          // Walk-back clipped to 24h: only today's 05:00Z fires, never older ones.
        let due = due_ticks(&expr, now - 3 * CATCHUP_WINDOW_MS, now);
        assert_eq!(due, vec![1_799_989_200_000]); // 2027-01-15T05:00:00Z
                                                  // One day earlier the window starts exactly at that fire, which is
                                                  // exclusive: nothing due.
        let earlier = 1_799_989_200_000;
        assert!(due_ticks(&expr, earlier, earlier).is_empty());
    }

    #[test]
    fn at_or_after_now_yields_no_ticks() {
        let expr = CronExpr::parse("* * * * *", None).unwrap();
        let now = NOW_MS;
        assert!(due_ticks(&expr, now, now).is_empty());
        assert!(due_ticks(&expr, now + 3_600_000, now).is_empty());
    }
}
