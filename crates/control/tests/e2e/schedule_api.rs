//! `/api/schedules` + the control-plane cron scheduler over the real router:
//! store-backed definition CRUD (schema v27 — the libsql `schedules` table
//! is the source of truth), the one-time `schedules.json` seed import, a
//! live agent fire with ledger history, `overlap: skip` gating on the last
//! run's execution state, catch-up missing ticks, and the admin-only
//! surface. `scan_interval_secs` stays a file ops knob (hot-read).

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

/// GET /api/schedules lists the STORED definitions (created via the admin
/// API), echoes every field, stamps created_at/updated_at, computes next_run
/// only for parseable crons, and reports the file ops knob `scan_interval_secs`.
#[tokio::test]
async fn lists_definitions_fail_soft() {
    let h = Harness::new().await;
    write_schedules(&h, &json!({"schedules": [], "scan_interval_secs": 7}));
    let (status, body) = create_schedule(
        &h,
        json!({
            "id": "pinger", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
            "params": {"prompt": "daily {{now-1d:%Y-%m-%d}}"}
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("pinger"));

    // Parked definitions skip the cron check at the door (enable flips it
    // back on), so an unparseable cron still stores and lists inert.
    let (status, body) = create_schedule(
        &h,
        json!({
            "id": "parked", "cron": "not a cron", "kind": "agent", "target": "act",
            "enabled": false
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 2);
    let pinger = schedules.iter().find(|s| s["id"] == "pinger").unwrap();
    assert_eq!(pinger["kind"], json!("agent"));
    assert_eq!(pinger["enabled"], json!(true));
    assert_eq!(
        pinger["params"]["prompt"],
        json!("daily {{now-1d:%Y-%m-%d}}")
    );
    assert!(pinger["next_run"].as_i64().unwrap() > 0, "{}", pinger);
    assert!(pinger["created_at"].as_i64().unwrap() > 0, "{}", pinger);
    assert!(pinger["updated_at"].as_i64().unwrap() > 0, "{}", pinger);
    let parked = schedules.iter().find(|s| s["id"] == "parked").unwrap();
    assert_eq!(parked["enabled"], json!(false));
    assert!(
        parked["next_run"].is_null(),
        "bad cron → no next tick: {parked}"
    );
    assert_eq!(body["scan_interval_secs"], json!(7));
}

/// Full CRUD surface: create (auto id), duplicate → 409, invalid → 400,
/// PUT full update (created_at preserved), PATCH enable/disable, DELETE.
#[tokio::test]
async fn schedule_crud_round_trip() {
    let h = Harness::new().await;

    // Create with an explicit id.
    let (status, body) = create_schedule(
        &h,
        json!({"id": "crud_job", "cron": USER_AGENT_CRON, "kind": "agent",
               "target": "act", "params": {"prompt": "v1"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("crud_job"));

    // Duplicate id → 409 (updates go through PUT).
    let (status, body) = create_schedule(
        &h,
        json!({"id": "crud_job", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
    )
    .await;
    assert_eq!(status, 409, "{body}");

    // A missing id gets a generated one.
    let (status, body) = create_schedule(
        &h,
        json!({"cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let auto_id = body["id"].as_str().unwrap();
    assert!(auto_id.starts_with("schedule-"), "{body}");

    // Invalid bodies are rejected at the door: empty target, unknown id
    // characters, an enabled job with a broken cron, brain params missing
    // the objective contract.
    for invalid in [
        json!({"id": "no_target", "cron": USER_AGENT_CRON, "kind": "agent"}),
        json!({"id": "has.dot", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"}),
        json!({"id": "bad_cron", "cron": "not a cron", "kind": "agent", "target": "act"}),
        json!({"id": "brain_no_obj", "cron": USER_AGENT_CRON, "kind": "brain",
               "target": "review_plan", "params": {}}),
    ] {
        let (status, body) = create_schedule(&h, invalid.clone()).await;
        assert_eq!(status, 400, "{invalid}: {body}");
    }

    let created_at = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()["created_at"]
        .as_i64()
        .unwrap();

    // PUT full update: cron + params change, created_at stays.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let (status, body) = h
        .req(
            reqwest::Method::PUT,
            "/api/schedules/crud_job",
            Some(
                json!({"cron": "* * * * *", "kind": "agent", "target": "act",
                        "params": {"prompt": "v2"}}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let job = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()
        .clone();
    assert_eq!(job["cron"], json!("* * * * *"));
    assert_eq!(job["params"]["prompt"], json!("v2"));
    assert_eq!(job["created_at"].as_i64(), Some(created_at));
    assert!(job["updated_at"].as_i64().unwrap() >= created_at);

    // PUT on an unknown id → 404 (create goes through POST).
    let (status, body) = h
        .req(
            reqwest::Method::PUT,
            "/api/schedules/ghost",
            Some(json!({"cron": USER_AGENT_CRON, "kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // PATCH enable/disable only; an unknown id → 404.
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/crud_job",
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let job = h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "crud_job")
        .unwrap()
        .clone();
    assert_eq!(job["enabled"], json!(false));
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/ghost",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Enabling a parked-with-broken-cron definition fails at the door.
    create_schedule(
        &h,
        json!({"id": "parked_bad", "cron": "not a cron", "kind": "agent",
               "target": "act", "enabled": false}),
    )
    .await;
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/parked_bad",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // DELETE removes the definition; the fire ledger stays queryable.
    let (status, body) = h
        .req(reqwest::Method::DELETE, "/api/schedules/crud_job", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(reqwest::Method::DELETE, "/api/schedules/ghost", None)
        .await;
    assert_eq!(status, 404, "{body}");
    let listed = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    assert!(
        !listed["schedules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == "crud_job"),
        "{listed}"
    );
    let (status, body) = h
        .req(reqwest::Method::GET, "/api/schedules/crud_job/runs", None)
        .await;
    assert_eq!(status, 200, "history survives the delete: {body}");
}

/// A definition created through the API (no seed file) is picked up by the
/// scheduler from the store and fires; PATCH disable stops the ticks.
#[tokio::test]
async fn created_definition_fires_then_patch_disables() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let (status, body) = create_schedule(
        &h,
        json!({"id": "api_pinger", "cron": USER_AGENT_CRON, "kind": "agent",
               "target": "act", "params": {"prompt": "ping"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let body = poll_runs(&h, "api_pinger", |b| {
        b["runs"].as_array().is_some_and(|runs| runs.len() >= 2)
    })
    .await;
    let runs = body["runs"].as_array().unwrap();
    // The first scan also records the pre-history as one collapsed `missed`
    // catch-up row next to the fresh fire.
    let fired = runs
        .iter()
        .find(|r| r["status"] == "fired")
        .expect("at least one fired tick");
    assert_eq!(fired["missed"], json!(false), "{body}");
    let execution_id = fired["execution_id"].as_str().unwrap();
    assert!(
        execution_id.starts_with("agent-api_pinger-"),
        "deterministic id: {execution_id}"
    );

    // Disable → after a settle, no new fires land.
    let (status, body) = h
        .req(
            reqwest::Method::PATCH,
            "/api/schedules/api_pinger",
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("api_pinger"));
    assert_eq!(
        h.req(reqwest::Method::GET, "/api/schedules", None).await.1["schedules"][0]["enabled"],
        json!(false)
    );
    assert_quiet(&h, "api_pinger", Duration::from_secs(4)).await;
}

/// dag schedules take `params.args` (appended to every wasm step's command
/// line at fire time): create must accept it (previously any dag params were
/// a 400) and the fire must land as a `fired` ledger row with the
/// deterministic dag- id.
#[tokio::test]
async fn dag_schedule_with_args_creates_and_fires() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    // The dag fire path resolves the definition by target; seed one first.
    let spec = json!({"name": "etl-args", "steps": [
        {"name": "fetch", "kind": {"type": "wasm", "command": "tool.wasm"}},
    ]});
    let (status, body) = h
        .req(
            reqwest::Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": spec})),
        )
        .await;
    assert_eq!(status, 200, "seed dag def: {body}");

    let (status, body) = create_schedule(
        &h,
        json!({"id": "dag_args", "cron": USER_AGENT_CRON, "kind": "dag",
               "target": "etl-args", "params": {"args": "--date 2026-09-18"}}),
    )
    .await;
    assert_eq!(status, 200, "create dag schedule with args: {body}");

    let body = poll_runs(&h, "dag_args", |b| {
        b["runs"]
            .as_array()
            .is_some_and(|runs| runs.iter().any(|r| r["status"] == "fired"))
    })
    .await;
    let fired = body["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == "fired")
        .expect("at least one fired tick: {body}");
    let execution_id = fired["execution_id"].as_str().unwrap();
    assert!(
        execution_id.starts_with("dag-dag_args-"),
        "deterministic id: {execution_id}"
    );
}

/// DELETE stops the ticks but the ledger history remains queryable.
#[tokio::test]
async fn delete_stops_firing_but_keeps_history() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let (status, body) = create_schedule(
        &h,
        json!({"id": "doomed", "cron": USER_AGENT_CRON, "kind": "agent",
               "target": "act", "params": {"prompt": "ping"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    poll_runs(&h, "doomed", |b| {
        b["runs"].as_array().is_some_and(|runs| !runs.is_empty())
    })
    .await;

    let (status, body) = h
        .req(reqwest::Method::DELETE, "/api/schedules/doomed", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let history = runs_of(&h, "doomed").await;
    assert!(
        !history["runs"].as_array().unwrap().is_empty(),
        "fire history survives the delete: {history}"
    );
    assert_quiet(&h, "doomed", Duration::from_secs(4)).await;
}

/// POST /:id/run fires immediately through the same submit path, records a
/// normal ledger row, and deliberately bypasses `enabled` (operator action).
#[tokio::test]
async fn manual_run_fires_now_even_when_disabled() {
    let h = Harness::new().await;
    let (status, body) = create_schedule(
        &h,
        json!({"id": "manual", "cron": "0 0 1 1 *", "kind": "agent", "target": "act",
               "params": {"prompt": "ping"}, "enabled": false}),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(reqwest::Method::POST, "/api/schedules/manual/run", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["scheduled_for_ms"].as_i64().unwrap() > 0);
    let execution_id = body["execution_id"].as_str().unwrap();
    assert!(execution_id.starts_with("agent-manual-"), "{body}");

    let runs = runs_of(&h, "manual").await;
    let fired = runs["runs"].as_array().unwrap();
    assert_eq!(fired.len(), 1, "{runs}");
    assert_eq!(fired[0]["status"], json!("fired"));
    assert_eq!(fired[0]["execution_id"], json!(execution_id));
    assert_eq!(fired[0]["missed"], json!(false));

    // The manual fire doubles as the scan cursor in the listing.
    let listed = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    let def = listed["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "manual")
        .unwrap();
    assert_eq!(def["enabled"], json!(false));
    assert_eq!(def["last_run"]["status"], json!("fired"));

    // Unknown id → 404.
    let (status, body) = h
        .req(reqwest::Method::POST, "/api/schedules/ghost/run", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

/// `overlap: skip`: while the previous fire's execution is non-terminal the
/// scheduler records nothing; once it lands in a terminal state the next
/// tick fires again.
#[tokio::test]
async fn overlap_skip_waits_for_terminal_last_run() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let (status, body) = create_schedule(
        &h,
        json!({"id": "ovl", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act",
               "params": {"prompt": "ping"}, "overlap": "skip"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
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
    write_fast_scan(&h);
    let (status, body) = create_schedule(
        &h,
        json!({"id": "catchup", "cron": "* * * * *", "kind": "agent", "target": "act",
               "params": {"prompt": "ping"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
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

/// The legacy `schedules.json` degrades to a one-time seed: an empty table
/// imports every valid entry (invalid ones warn + skip, the old fail-soft
/// contract), and once definitions exist a mutated file is dead weight — a
/// restart must never resurrect a deleted or stale definition.
#[tokio::test]
async fn file_seed_is_one_time_and_table_gated() {
    let h = Harness::new().await;
    let (status, body) = h.req(reqwest::Method::GET, "/api/schedules", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["schedules"].as_array().unwrap().is_empty(),
        "no file, no definitions: {body}"
    );

    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "seeded", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "ping"}},
                {"id": "broken", "cron": "not a cron", "kind": "agent", "target": "act"}
            ],
            "scan_interval_secs": 1
        }),
    );
    opencoder_control::seed_schedules::seed_schedules(&h.state.store, &h.state.workdir).await;
    let body = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 1, "invalid entries are skipped: {body}");
    assert_eq!(schedules[0]["id"], json!("seeded"));
    assert_eq!(schedules[0]["cron"], json!(USER_AGENT_CRON));
    assert!(schedules[0]["created_at"].as_i64().unwrap() > 0);
    assert_eq!(body["scan_interval_secs"], json!(1));

    // A mutated file does not re-import: the table is no longer empty.
    write_schedules(
        &h,
        &json!({
            "schedules": [
                {"id": "seeded", "cron": "* * * * *", "kind": "agent",
                 "target": "renamed", "params": {"prompt": "changed"}},
                {"id": "latecomer", "cron": USER_AGENT_CRON, "kind": "agent",
                 "target": "act", "params": {"prompt": "new"}}
            ],
            "scan_interval_secs": 1
        }),
    );
    opencoder_control::seed_schedules::seed_schedules(&h.state.store, &h.state.workdir).await;
    let body = h.req(reqwest::Method::GET, "/api/schedules", None).await.1;
    let schedules = body["schedules"].as_array().unwrap();
    assert_eq!(schedules.len(), 1, "one-time import, no merge: {body}");
    assert_eq!(schedules[0]["cron"], json!(USER_AGENT_CRON), "{body}");
    assert_eq!(schedules[0]["target"], json!("act"), "{body}");
}

/// A brain job whose params miss `objective` is structurally invalid:
/// `ScheduleJob::validate` rejects it at the door (create → 400), and a
/// healthy sibling in the same batch keeps firing (fail-soft isolation).
#[tokio::test]
async fn brain_schedule_without_objective_is_rejected_without_starving_siblings() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let (status, body) = create_schedule(
        &h,
        json!({"id": "brain_no_obj", "cron": USER_AGENT_CRON, "kind": "brain",
               "target": "review_plan", "params": {}}),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = create_schedule(
        &h,
        json!({"id": "healthy", "cron": USER_AGENT_CRON, "kind": "agent",
               "target": "act", "params": {"prompt": "ping"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
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
}

/// The schedules surface (reads AND writes) is admin-only (unknown paths
/// default to closed).
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

    // Write paths are gated the same way.
    let body = json!({"id": "evil", "cron": USER_AGENT_CRON, "kind": "agent", "target": "act"});
    for (method, path, payload) in [
        (reqwest::Method::POST, "/api/schedules", Some(body.clone())),
        (
            reqwest::Method::PUT,
            "/api/schedules/evil",
            Some(body.clone()),
        ),
        (
            reqwest::Method::PATCH,
            "/api/schedules/evil",
            Some(json!({"enabled": false})),
        ),
        (reqwest::Method::DELETE, "/api/schedules/evil", None),
        (reqwest::Method::POST, "/api/schedules/evil/run", None),
    ] {
        let resp = h.req_raw(method, path, payload, Some(&token)).await;
        assert_eq!(resp.status().as_u16(), 403, "{path}");
    }
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
