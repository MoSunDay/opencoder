use super::*;
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus};

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
    let created_at = h
        .state
        .store
        .get_schedule("api_pinger")
        .await
        .unwrap()
        .unwrap()
        .created_at;

    let body = poll_runs(&h, "api_pinger", |b| {
        b["runs"]
            .as_array()
            .is_some_and(|runs| runs.iter().any(|r| r["status"] == "fired"))
    })
    .await;
    let runs = body["runs"].as_array().unwrap();
    assert!(
        runs.iter()
            .all(|run| run["scheduled_for_ms"].as_i64().unwrap() > created_at),
        "a new schedule must not fire or record missed ticks from before creation: {body}"
    );
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

/// dag schedules take `params.args` (appended to every binary step's command
/// line at fire time): create must accept it (previously any dag params were
/// a 400) and the fire must land as a `fired` ledger row with the
/// deterministic dag- id.
#[tokio::test]
async fn dag_schedule_with_args_creates_and_fires() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    // The dag fire path resolves the definition by target; seed one first.
    let spec = json!({"name": "etl-args", "steps": [
        {"name": "fetch", "kind": {"type":"binary","resource":"tool"}},
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
               "target": "etl-args", "params": {"args": ["--date", "2026-09-18"]}}),
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
    // Last fire two hours ago (execution long finished): every minute tick
    // since then is due, inside the 24h catch-up window. The definition
    // already existed before that fire, as it would after a server restart.
    let now = opencoder_core::message::now_ms();
    let last = now - 2 * 60 * 60 * 1000;
    let job = serde_json::from_value(json!({"id": "catchup", "cron": "* * * * *",
        "kind": "agent", "target": "act", "params": {"prompt": "ping"}}))
    .unwrap();
    h.state
        .store
        .upsert_schedule(&job, last - 60_000)
        .await
        .unwrap();
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
