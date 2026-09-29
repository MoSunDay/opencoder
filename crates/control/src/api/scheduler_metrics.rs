//! Authenticated, low-cardinality view of the control-plane scheduler.
use crate::{scheduler::telemetry::SchedulerCounters, AppState};
use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, StatusCode},
    response::{IntoResponse, Response},
};
use opencoder_core::{
    fleet::{ExecutionStatus, NodeView},
    message::now_ms,
    schedule::{to_utc, CronExpr},
};
use opencoder_store::{
    ScheduleDefRecord, ScheduleRunRecord, SCHEDULE_RUN_ERROR, SCHEDULE_RUN_FIRED,
    SCHEDULE_RUN_MISSED,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Default)]
struct LastExecutions {
    active: u64,
    done: u64,
    failed: u64,
}

struct CurrentLoad {
    inflight_admissions: u64,
    active_executions: u64,
}

#[derive(Debug, Serialize)]
pub struct SchedulerSnapshot {
    pub observed_at_ms: i64,
    pub scans_total: u64,
    pub scan_errors_total: u64,
    pub last_scan_started_ms: i64,
    pub last_scan_completed_ms: i64,
    pub fire_attempts_total: u64,
    pub fire_errors_total: u64,
    pub missed_ticks_total: u64,
    pub schedules_total: u64,
    pub schedules_enabled: u64,
    pub schedules_last_fire_error: u64,
    pub schedules_last_fire_missed: u64,
    pub schedules_last_execution_active: u64,
    pub schedules_last_execution_done: u64,
    pub schedules_last_execution_failed: u64,
    pub next_tick_ms: i64,
    pub nodes_online: u64,
    pub nodes_ready: u64,
    pub node_capacity: u64,
    pub node_active_runs: u64,
    pub node_pending_runs: u64,
    pub node_reserved_runs: u64,
    pub inflight_admissions: u64,
    pub active_executions: u64,
}

fn summarize(
    now: i64,
    definitions: &[ScheduleDefRecord],
    last_runs: &[Option<ScheduleRunRecord>],
    nodes: &[NodeView],
    counters: SchedulerCounters,
    executions: LastExecutions,
    load: CurrentLoad,
) -> SchedulerSnapshot {
    let next_tick_ms = definitions
        .iter()
        .filter(|def| def.job.enabled)
        .filter_map(|def| {
            CronExpr::parse(&def.job.cron, def.job.timezone.as_deref())
                .ok()?
                .next_after(to_utc(now))
                .map(|tick| tick.timestamp_millis())
        })
        .min()
        .unwrap_or(0);
    let online = nodes.iter().filter(|node| node.online).collect::<Vec<_>>();
    SchedulerSnapshot {
        observed_at_ms: now,
        scans_total: counters.scans_total,
        scan_errors_total: counters.scan_errors_total,
        last_scan_started_ms: counters.last_scan_started_ms,
        last_scan_completed_ms: counters.last_scan_completed_ms,
        fire_attempts_total: counters.fire_attempts_total,
        fire_errors_total: counters.fire_errors_total,
        missed_ticks_total: counters.missed_ticks_total,
        schedules_total: definitions.len() as u64,
        schedules_enabled: definitions.iter().filter(|def| def.job.enabled).count() as u64,
        schedules_last_fire_error: last_runs
            .iter()
            .flatten()
            .filter(|run| run.status == SCHEDULE_RUN_ERROR)
            .count() as u64,
        schedules_last_fire_missed: last_runs
            .iter()
            .flatten()
            .filter(|run| run.status == SCHEDULE_RUN_MISSED)
            .count() as u64,
        schedules_last_execution_active: executions.active,
        schedules_last_execution_done: executions.done,
        schedules_last_execution_failed: executions.failed,
        next_tick_ms,
        nodes_online: online.len() as u64,
        nodes_ready: online
            .iter()
            .filter(|node| node.snapshot.as_ref().is_some_and(|s| s.ready))
            .count() as u64,
        node_capacity: online
            .iter()
            .filter_map(|node| node.snapshot.as_ref())
            .map(|s| s.max_runs)
            .sum(),
        node_active_runs: online
            .iter()
            .filter_map(|node| node.snapshot.as_ref())
            .map(|s| s.active_runs)
            .sum(),
        node_pending_runs: online
            .iter()
            .filter_map(|node| node.snapshot.as_ref())
            .map(|s| s.pending_runs)
            .sum(),
        node_reserved_runs: online.iter().map(|node| node.reserved_loops).sum(),
        inflight_admissions: load.inflight_admissions,
        active_executions: load.active_executions,
    }
}

async fn collect(state: &AppState) -> anyhow::Result<SchedulerSnapshot> {
    let now = now_ms();
    let definitions = state.store.list_schedules().await?;
    let mut last_runs = Vec::with_capacity(definitions.len());
    let mut executions = LastExecutions::default();
    for def in &definitions {
        let run = state.store.last_schedule_run(&def.job.id).await?;
        if let Some(id) = run
            .as_ref()
            .filter(|run| run.status == SCHEDULE_RUN_FIRED)
            .and_then(|run| run.execution_id.as_deref())
        {
            if let Some(index) = state.fleet.index(id).await? {
                match index.status {
                    ExecutionStatus::Error | ExecutionStatus::Cancelled => executions.failed += 1,
                    status if !status.terminal() => executions.active += 1,
                    ExecutionStatus::Done => executions.done += 1,
                    _ => {}
                }
            }
        }
        last_runs.push(run);
    }
    let nodes = state.hub.views().await;
    let admission = state.admission.snapshot().await?;
    let active_executions = state.fleet.active_execution_count().await?;
    Ok(summarize(
        now,
        &definitions,
        &last_runs,
        &nodes,
        state.lifecycle.scheduler.snapshot(),
        executions,
        CurrentLoad {
            inflight_admissions: admission.inflight_admissions,
            active_executions,
        },
    ))
}

pub async fn json(State(state): State<Arc<AppState>>) -> Response {
    match collect(&state).await {
        Ok(snapshot) => super::response(opencoder_core::fleet::RpcReply::ok(
            serde_json::to_value(snapshot).expect("scheduler metrics are finite"),
        )),
        Err(error) => super::error_500(format!("scheduler metrics: {error:#}")),
    }
}

fn render_prometheus(snapshot: &SchedulerSnapshot) -> String {
    let mut output = String::new();
    let values: [(&str, &str, u64); 22] = [
        (
            "opencoder_scheduler_scans_total",
            "counter",
            snapshot.scans_total,
        ),
        (
            "opencoder_scheduler_scan_errors_total",
            "counter",
            snapshot.scan_errors_total,
        ),
        (
            "opencoder_scheduler_fire_attempts_total",
            "counter",
            snapshot.fire_attempts_total,
        ),
        (
            "opencoder_scheduler_fire_errors_total",
            "counter",
            snapshot.fire_errors_total,
        ),
        (
            "opencoder_scheduler_missed_ticks_total",
            "counter",
            snapshot.missed_ticks_total,
        ),
        (
            "opencoder_scheduler_schedules_total",
            "gauge",
            snapshot.schedules_total,
        ),
        (
            "opencoder_scheduler_schedules_enabled",
            "gauge",
            snapshot.schedules_enabled,
        ),
        (
            "opencoder_scheduler_schedules_last_fire_error",
            "gauge",
            snapshot.schedules_last_fire_error,
        ),
        (
            "opencoder_scheduler_schedules_last_fire_missed",
            "gauge",
            snapshot.schedules_last_fire_missed,
        ),
        (
            "opencoder_scheduler_schedules_last_execution_active",
            "gauge",
            snapshot.schedules_last_execution_active,
        ),
        (
            "opencoder_scheduler_schedules_last_execution_done",
            "gauge",
            snapshot.schedules_last_execution_done,
        ),
        (
            "opencoder_scheduler_schedules_last_execution_failed",
            "gauge",
            snapshot.schedules_last_execution_failed,
        ),
        (
            "opencoder_scheduler_nodes_online",
            "gauge",
            snapshot.nodes_online,
        ),
        (
            "opencoder_scheduler_nodes_ready",
            "gauge",
            snapshot.nodes_ready,
        ),
        (
            "opencoder_scheduler_node_capacity",
            "gauge",
            snapshot.node_capacity,
        ),
        (
            "opencoder_scheduler_node_active_runs",
            "gauge",
            snapshot.node_active_runs,
        ),
        (
            "opencoder_scheduler_node_pending_runs",
            "gauge",
            snapshot.node_pending_runs,
        ),
        (
            "opencoder_scheduler_node_reserved_runs",
            "gauge",
            snapshot.node_reserved_runs,
        ),
        (
            "opencoder_scheduler_inflight_admissions",
            "gauge",
            snapshot.inflight_admissions,
        ),
        (
            "opencoder_scheduler_active_executions",
            "gauge",
            snapshot.active_executions,
        ),
        (
            "opencoder_scheduler_last_scan_timestamp_seconds",
            "gauge",
            (snapshot.last_scan_completed_ms / 1000).max(0) as u64,
        ),
        (
            "opencoder_scheduler_next_tick_timestamp_seconds",
            "gauge",
            (snapshot.next_tick_ms / 1000).max(0) as u64,
        ),
    ];
    for (name, kind, value) in values {
        output.push_str(&format!("# TYPE {name} {kind}\n{name} {value}\n"));
    }
    output
}

pub async fn prometheus(State(state): State<Arc<AppState>>) -> Response {
    match collect(&state).await {
        Ok(snapshot) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
            render_prometheus(&snapshot),
        )
            .into_response(),
        Err(error) => super::error_500(format!("scheduler metrics: {error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_has_explicit_zeroes_and_no_unbounded_labels() {
        let snapshot = summarize(
            42,
            &[],
            &[],
            &[],
            crate::scheduler::telemetry::SchedulerTelemetry::default().snapshot(),
            LastExecutions::default(),
            CurrentLoad {
                inflight_admissions: 0,
                active_executions: 0,
            },
        );
        assert_eq!(snapshot.schedules_total, 0);
        assert_eq!(snapshot.next_tick_ms, 0);
        let body = render_prometheus(&snapshot);
        assert!(body.contains("opencoder_scheduler_schedules_enabled 0\n"));
        assert!(body.contains("opencoder_scheduler_last_scan_timestamp_seconds 0\n"));
        assert!(!body.contains("schedule_id="));
    }
}
