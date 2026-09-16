//! `/api/schedules` + the control-plane cron scheduler over the real router:
//! definition listing, a live agent fire with ledger history, `overlap:
//! skip` gating on the last run's execution state, catch-up missing ticks,
//! error recording, and the admin-only surface.

use serde_json::{json, Value};
use std::time::Duration;

use crate::support::http::Harness;
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};
use opencoder_store::{ScheduleRunRecord, SCHEDULE_RUN_FIRED};

const USER_AGENT_CRON: &str = "*/1 * * * * *"; // 6-field: every second

/// `schedules.json` is a domain file under `<workdir>/.opencoder/`.
fn write_schedules(h: &Harness, body: &Value) {
    let dir = h.state.workdir.join(".opencoder");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("schedules.json"), body.to_string()).unwrap();
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

/// Poll the runs endpoint until `want` holds, for the failure message.
async fn poll_runs(h: &Harness, id: &str, want: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let body = runs_of(h, id).await;
        if want(&body) || tokio::time::Instant::now() >= deadline {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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

/// GET /api/schedules echoes the configured definitions (including disabled
/// ones) and computes next_run only for parseable crons.
#[tokio::test]
async fn lists_definitions_fail_soft() {
    let h = Harness::new().await;
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "pinger", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
                 "params": {"prompt": "daily {{now-1d:%Y-%m-%d}}"}},
                {"id": "parked", "cron": "not a cron", "kind": "agent", "target": "act",
                 "enabled": false}
            ],
            "scan_interval_secs": 1
        }),
    );
    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 2);
    assert_eq!(schedules[0]["id"], json!("pinger"));
    assert_eq!(schedules[0]["kind"], json!("agent"));
    assert_eq!(schedules[0]["enabled"], json!(true));
    assert_eq!(
        schedules[0]["params"]["prompt"],
        json!("daily {{now-1d:%Y-%m-%d}}")
    );
    assert!(
        schedules[0]["next_run"].as_i64().unwrap() > 0,
        "next tick is computed: {}",
        schedules[0]
    );
    assert_eq!(schedules[1]["id"], json!("parked"));
    assert_eq!(schedules[1]["enabled"], json!(false));
    assert!(
        schedules[1]["next_run"].is_null(),
        "bad cron → no next tick"
    );
    assert_eq!(body["scan_interval_secs"], json!(1));
}

/// A per-second agent job fires through the real submit path; the ledger
/// records the deterministic execution id and /api/schedules reflects it.
#[tokio::test]
async fn scheduler_fires_an_agent_tick_and_records_history() {
    let h = Harness::new().await;
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "nightly_pinger", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "ping"}}
            ],
            "scan_interval_secs": 1
        }),
    );
    let body = poll_runs(&h, "nightly_pinger", |b| {
        b["runs"]
            .as_array()
            .is_some_and(|runs| runs.iter().any(|r| r["status"] == "fired"))
    })
    .await;
    let runs = body["runs"].as_array().unwrap();
    let fired = runs
        .iter()
        .find(|r| r["status"] == "fired")
        .expect("at least one fired tick");
    let execution_id = fired["execution_id"].as_str().unwrap();
    assert!(
        execution_id.starts_with("agent-nightly_pinger-"),
        "deterministic id: {execution_id}"
    );
    assert_eq!(fired["kind"], json!("agent"));
    assert_eq!(fired["target"], json!("act"));
    assert_eq!(fired["missed"], json!(false));
    assert!(fired["scheduled_for_ms"].as_i64().unwrap() > 0);

    // The fired execution exists in the control index.
    let index = h.state.fleet.index(execution_id).await.unwrap().unwrap();
    assert_eq!(index.kind, ExecutionKind::Agent);

    // /api/schedules surfaces the ledger row as last_run.
    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    let pinger = body["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "nightly_pinger")
        .unwrap();
    assert_eq!(pinger["last_run"]["status"], json!("fired"));
}

/// `overlap: skip`: while the previous fire's execution is non-terminal the
/// scheduler records nothing; once it lands in a terminal state the next
/// tick fires again.
#[tokio::test]
async fn overlap_skip_waits_for_terminal_last_run() {
    let h = Harness::new().await;
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "ovl", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
                 "params": {"prompt": "ping"}, "overlap": "skip"}
            ],
            "scan_interval_secs": 1
        }),
    );
    let now = opencoder_core::message::now_ms();
    let execution_id = "agent-ovl-running";
    h.state
        .store
        .record_schedule_run(&fired_run("ovl", now, execution_id))
        .await
        .unwrap();
    h.put_index(execution_id, ExecutionKind::Agent, ExecutionStatus::Running)
        .await;

    // A couple of scans pass: the running execution suppresses new fires.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let body = runs_of(&h, "ovl").await;
    assert_eq!(
        body["runs"].as_array().unwrap().len(),
        1,
        "no new ticks while the previous fire runs: {body}"
    );

    // Land the running execution in a terminal state (same owner fields —
    // put_index only accepts a status flip on identical ownership).
    let index = h.state.fleet.index(execution_id).await.unwrap().unwrap();
    h.state
        .fleet
        .put_index(&ExecutionIndex {
            id: index.id.clone(),
            created_at: index.created_at,
            kind: index.kind,
            node_id: index.node_id.clone(),
            status: ExecutionStatus::Done,
        })
        .await
        .unwrap();
    let body = poll_runs(&h, "ovl", |b| {
        b["runs"].as_array().is_some_and(|runs| {
            runs.iter()
                .any(|r| r["status"] == "fired" && r["execution_id"] != execution_id)
        })
    })
    .await;
    assert!(
        body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "fired" && r["execution_id"] != execution_id),
        "a later tick fired after the previous run finished: {body}"
    );
}

/// Catch-up: ticks older than the most recent due one are recorded as
/// `missed` (audit trail), and only the newest tick fires.
#[tokio::test]
async fn catch_up_records_older_ticks_as_missed() {
    let h = Harness::new().await;
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "catchup", "cron": "* * * * *", "kind": "agent", "target": "act",
                 "params": {"prompt": "ping"}}
            ],
            "scan_interval_secs": 1
        }),
    );
    // Last fire two hours ago (execution long finished): every minute tick
    // since then is due, inside the 24h catch-up window.
    let now = opencoder_core::message::now_ms();
    let last = now - 2 * 60 * 60 * 1000;
    let execution_id = "agent-catchup-old";
    h.state
        .store
        .record_schedule_run(&fired_run("catchup", last, execution_id))
        .await
        .unwrap();
    h.put_index(execution_id, ExecutionKind::Agent, ExecutionStatus::Done)
        .await;

    let body = poll_runs(&h, "catchup", |b| {
        b["runs"].as_array().is_some_and(|runs| {
            runs.iter()
                .any(|r| r["status"] == "fired" && r["execution_id"] != execution_id)
        })
    })
    .await;
    let runs = body["runs"].as_array().unwrap();
    let fired: Vec<_> = runs
        .iter()
        .filter(|r| r["status"] == "fired" && r["execution_id"] != execution_id)
        .collect();
    let missed: Vec<_> = runs.iter().filter(|r| r["missed"] == true).collect();
    assert_eq!(fired.len(), 1, "exactly the newest tick fires: {body}");
    assert_eq!(
        missed.len(),
        1,
        "the skipped window collapses into one representative row: {body}"
    );
    let fire_for = fired[0]["scheduled_for_ms"].as_i64().unwrap();
    assert!(
        missed[0]["scheduled_for_ms"].as_i64().unwrap() < fire_for,
        "the missed representative predates the fired tick"
    );
    assert!(missed[0]["execution_id"].is_null());
}

/// A brain job whose params miss `objective` is structurally invalid:
/// `ScheduleJob::validate` rejects it at config time, so the scheduler
/// skips it before any fire — no ledger rows ever appear, the definition
/// still lists on the surface (last_run stays null), and a healthy
/// sibling in the same file keeps firing (fail-soft isolation).
#[tokio::test]
async fn brain_schedule_without_objective_is_skipped_without_starving_siblings() {
    let h = Harness::new().await;
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "brain_no_obj", "cron": USER_AGENT_CRON, "kind": "brain",
                 "target": "review_plan", "params": {}},
                {"id": "healthy", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "ping"}}
            ],
            "scan_interval_secs": 1
        }),
    );
    let body = poll_runs(&h, "healthy", |b| {
        b["runs"]
            .as_array()
            .is_some_and(|runs| runs.iter().any(|r| r["status"] == "fired"))
    })
    .await;
    assert!(
        body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "fired"),
        "the valid sibling keeps firing: {body}"
    );
    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    let listed = body["schedules"].as_array().unwrap();
    let invalid = listed
        .iter()
        .find(|s| s["id"] == json!("brain_no_obj"))
        .expect("the definition is still listed: {body}");
    assert!(
        invalid["last_run"].is_null(),
        "a config-invalid job never fires, so no ledger row: {invalid}"
    );
    assert!(
        invalid["next_run"].as_i64().unwrap() > 0,
        "the cron itself is fine, so the next tick is computed: {invalid}"
    );
    let body = runs_of(&h, "brain_no_obj").await;
    assert!(
        body["runs"].as_array().is_some_and(|runs| runs.is_empty()),
        "no dispatch, no error row: {body}"
    );
}

/// The schedules surface is admin-only (unknown paths default to closed).
#[tokio::test]
async fn schedule_apis_are_admin_only() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            reqwest::Method::POST,
            "/api/users",
            Some(json!({"name": "alice", "role": "user"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let token = body["token"].as_str().unwrap().to_string();

    let resp = h
        .req_raw(reqwest::Method::GET, "/api/schedules", None, Some(&token))
        .await;
    assert_eq!(resp.status().as_u16(), 403);
    let resp = h
        .req_raw(
            reqwest::Method::GET,
            "/api/schedules/whatever/runs",
            None,
            Some(&token),
        )
        .await;
    assert_eq!(resp.status().as_u16(), 403);
}

/// Malformed schedule ids are rejected before hitting the store.
#[tokio::test]
async fn runs_rejects_invalid_ids() {
    let h = Harness::new().await;
    let too_long = "x".repeat(41);
    for id in ["has.dot", "has%20space", too_long.as_str()] {
        let (status, body) = h
            .req(
                reqwest::Method::GET,
                &format!("/api/schedules/{id}/runs"),
                None,
            )
            .await;
        assert_eq!(status, 400, "{id}: {body}");
    }
}
