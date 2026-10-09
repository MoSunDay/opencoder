//! `/api/schedules` + the control-plane cron scheduler over the real router:
//! store-backed definition CRUD (schema v27 — the libsql `schedules` table
//! is the source of truth), the one-time `schedules.json` seed import, a
//! live agent fire with ledger history, `overlap: skip` gating on the last
//! run's execution state, catch-up missing ticks, and the admin-only
//! surface. `scan_interval_secs` stays a file ops knob (hot-read).

use serde_json::{json, Value};
use std::time::Duration;

use crate::support::http::Harness;
use opencoder_store::{ScheduleRunRecord, SCHEDULE_RUN_FIRED};

const USER_AGENT_CRON: &str = "*/1 * * * * *"; // 6-field: every second

/// `schedules.json` is a domain file under `<workdir>/.opencoder/`.
fn write_schedules(h: &Harness, body: &Value) {
    let dir = h.state.workdir.join(".opencoder");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("schedules.json"), body.to_string()).unwrap();
}

/// The file still owns the scheduler cadence (the table never stores it), so
/// firing tests ship an empty-definition file with a 1s scan and create
/// their definitions through the admin API.
fn write_fast_scan(h: &Harness) {
    write_schedules(h, &json!({"schedules": [], "scan_interval_secs": 1}));
}

async fn create_schedule(h: &Harness, body: Value) -> (reqwest::StatusCode, Value) {
    h.req(reqwest::Method::POST, "/api/schedules", Some(body))
        .await
}

async fn runs_of(h: &Harness, id: &str) -> Value {
    let (status, body) = h
        .req(
            reqwest::Method::GET,
            &format!("/api/schedules/{id}/runs"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn fired_count(h: &Harness, id: &str) -> usize {
    runs_of(h, id).await["runs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["status"] == "fired")
        .count()
}

/// Poll the runs endpoint until `want` holds, for the failure message.
async fn poll_runs(h: &Harness, id: &str, want: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        let body = runs_of(h, id).await;
        if want(&body) || tokio::time::Instant::now() >= deadline {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Let any fire that raced a disable/delete land, then prove quietness.
async fn assert_quiet(h: &Harness, id: &str, wait: Duration) {
    tokio::time::sleep(Duration::from_secs(2)).await; // in-flight settle
    let before = fired_count(h, id).await;
    tokio::time::sleep(wait).await;
    let after = fired_count(h, id).await;
    assert_eq!(before, after, "no new fires after the definition change");
}

fn fired_run(schedule: &str, for_ms: i64, execution_id: &str) -> ScheduleRunRecord {
    ScheduleRunRecord {
        schedule_id: schedule.to_string(),
        kind: "agent".into(),
        target: "act".into(),
        scheduled_for_ms: for_ms,
        fired_at_ms: for_ms,
        execution_id: Some(execution_id.to_string()),
        status: SCHEDULE_RUN_FIRED.into(),
        error: None,
        missed: false,
    }
}

mod definitions;
mod firing;
mod isolation;
mod timing;
