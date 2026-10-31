mod common;
use common::{call, Fixture};
use opencoder_core::identity::Role;
use opencoder_ontology::{
    FreshnessItemStatus, FreshnessOperation as Op, FreshnessRun, FreshnessRunStatus,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn policy_body(kind: &str) -> Value {
    json!({
        "name": "nightly refresh",
        "description": "refresh captured entities",
        "entity_type_id": kind,
        "executor": {"kind": "agent", "target": "agent://refresher", "prompt_template": "refresh {{name}}"},
        "concurrency": 5
    })
}

fn update_body(kind: &str, name: &str, revision: i64) -> Value {
    json!({
        "name": name,
        "description": "refresh captured entities",
        "entity_type_id": kind,
        "executor": {"kind": "agent", "target": "agent://refresher", "prompt_template": "refresh {{name}}"},
        "concurrency": 5,
        "writeback_attribute_key": null,
        "is_deleted": false,
        "expected_revision": revision
    })
}

#[tokio::test]
async fn policy_crud_enforces_validation_and_revisions() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let unknown = policy_body(&Uuid::new_v4().to_string());
    let (status, _) = f.call("POST", "/envs/debug/freshness/policies", unknown).await;
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    let mut blank_concurrency = policy_body(&kind);
    blank_concurrency["concurrency"] = json!(0);
    let mut excess_concurrency = policy_body(&kind);
    excess_concurrency["concurrency"] = json!(31);
    let mut agent_without_prompt = policy_body(&kind);
    agent_without_prompt["executor"]["prompt_template"] = json!(" ");
    for invalid in [blank_concurrency, excess_concurrency, agent_without_prompt] {
        let (status, body) = f.call("POST", "/envs/debug/freshness/policies", invalid).await;
        assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }
    let created = f.ok("POST", "/envs/debug/freshness/policies", policy_body(&kind)).await;
    let id = created["item"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["item"]["revision"], 0);
    assert_eq!(created["item"]["is_deleted"], false);
    let detail = f.ok("GET", &format!("/envs/debug/freshness/policies/{id}"), Value::Null).await;
    assert_eq!(detail["item"]["name"], "nightly refresh");
    let renamed = f
        .ok("PATCH", &format!("/envs/debug/freshness/policies/{id}"), update_body(&kind, "renamed", 0))
        .await;
    assert_eq!(renamed["revision"], 1);
    let (status, _) = f
        .call("PATCH", &format!("/envs/debug/freshness/policies/{id}"), update_body(&kind, "stale", 0))
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
    let mut deleted = update_body(&kind, "renamed", 1);
    deleted["is_deleted"] = json!(true);
    f.ok("PATCH", &format!("/envs/debug/freshness/policies/{id}"), deleted).await;
    let listed = f.ok("GET", "/envs/debug/freshness/policies", Value::Null).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 0, "soft-deleted policies hide by default");
    let with_deleted = f
        .ok("GET", "/envs/debug/freshness/policies?include_deleted=true", Value::Null)
        .await;
    assert_eq!(with_deleted["items"].as_array().unwrap().len(), 1);
    assert_eq!(with_deleted["items"][0]["is_deleted"], true);
}

async fn create_policy(f: &Fixture, kind: &str, writeback: Option<&str>) -> Value {
    let mut body = policy_body(kind);
    if let Some(key) = writeback {
        body["writeback_attribute_key"] = json!(key);
    }
    f.ok("POST", "/envs/debug/freshness/policies", body).await["item"].clone()
}

fn run_from(value: &Value) -> FreshnessRun {
    serde_json::from_value(value.clone()).unwrap()
}

async fn save_run(f: &Fixture, run: FreshnessRun, expected: i64) -> Value {
    f.state
        .freshness("debug".into(), Op::RunSave { run, expected_revision: expected })
        .await
        .unwrap()["item"]
        .clone()
}

#[tokio::test]
async fn run_lifecycle_settles_status_and_locks_scope() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let alpha = f.entity(&kind, "alpha").await;
    let beta = f.entity(&kind, "beta").await;
    let policy = create_policy(&f, &kind, None).await;
    let policy_id = Uuid::parse_str(policy["id"].as_str().unwrap()).unwrap();
    let run = f
        .state
        .freshness("debug".into(), Op::RunCreate { policy_id })
        .await
        .unwrap()["item"]
        .clone();
    assert_eq!(run["status"], "queued");
    assert_eq!(run["revision"], 0);
    assert_eq!(run["stats"], json!({"total": 2, "pending": 2, "dispatched": 0, "succeeded": 0, "failed": 0}));
    let run_id = run["id"].as_str().unwrap().to_string();
    let http = f
        .ok("GET", &format!("/envs/debug/freshness/runs/{run_id}"), Value::Null)
        .await;
    assert_eq!(http["item"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(http["item"]["items"][0]["status"], "pending");
    let listed = f.ok("GET", "/envs/debug/freshness/runs", Value::Null).await;
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    let filtered = f
        .ok("GET", &format!("/envs/debug/freshness/runs?policy_id={policy_id}"), Value::Null)
        .await;
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    let other = f
        .ok("GET", "/envs/debug/freshness/runs?policy_id=00000000-0000-4000-8000-000000000001", Value::Null)
        .await;
    assert_eq!(other["items"].as_array().unwrap().len(), 0);
    let envs = f.state.freshness("debug".into(), Op::Environments).await.unwrap();
    assert!(envs["items"].as_array().unwrap().iter().any(|item| item["env_key"] == "debug"));
    let entities = f
        .state
        .freshness("debug".into(), Op::EntitiesOfType {
            type_id: Uuid::parse_str(&kind).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(entities["items"].as_array().unwrap().len(), 2);

    let mut running = run_from(&run);
    running.status = FreshnessRunStatus::Running;
    for (index, item) in running.items.iter_mut().enumerate() {
        item.status = FreshnessItemStatus::Dispatched;
        item.execution_id = Some(format!("exec-{index}"));
    }
    let saved = save_run(&f, running, 0).await;
    assert_eq!(saved["status"], "running");
    assert_eq!(saved["revision"], 1);
    assert_eq!(saved["stats"]["dispatched"], 2);
    assert_eq!(
        f.state
            .freshness("debug".into(), Op::RunsActive)
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let mut rogue = run_from(&saved);
    rogue.items.pop();
    let rejected = f
        .state
        .freshness("debug".into(), Op::RunSave { run: rogue, expected_revision: 1 })
        .await;
    assert!(rejected.is_err(), "run scope is immutable");

    let mut finished = run_from(&saved);
    finished.status = FreshnessRunStatus::Partial;
    finished.items[0].status = FreshnessItemStatus::Succeeded;
    finished.items[0].candidate = Some("# refreshed\n\nbody".into());
    finished.items[1].status = FreshnessItemStatus::Failed;
    finished.items[1].error = Some("executor unavailable".into());
    let settled = save_run(&f, finished, 1).await;
    assert_eq!(settled["status"], "partial");
    assert_eq!(settled["revision"], 2);
    assert_eq!(settled["stats"], json!({"total": 2, "pending": 0, "dispatched": 0, "succeeded": 1, "failed": 1}));
    assert_eq!(
        f.state
            .freshness("debug".into(), Op::RunsActive)
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "terminal runs leave the active set"
    );
    let mut rewind = run_from(&settled);
    rewind.status = FreshnessRunStatus::Running;
    assert!(
        f.state
            .freshness("debug".into(), Op::RunSave { run: rewind, expected_revision: 2 })
            .await
            .is_err(),
        "terminal states are final"
    );
    assert_eq!(alpha["revision"], beta["revision"], "runs never touch entities");
}

async fn succeed_all(f: &Fixture, run: &Value) -> Value {
    let mut finished = run_from(run);
    finished.status = FreshnessRunStatus::Running;
    for item in finished.items.iter_mut() {
        item.status = FreshnessItemStatus::Dispatched;
        item.execution_id = Some("exec".into());
    }
    let running = save_run(f, finished, run["revision"].as_i64().unwrap()).await;
    let mut done = run_from(&running);
    done.status = FreshnessRunStatus::Succeeded;
    for item in done.items.iter_mut() {
        item.status = FreshnessItemStatus::Succeeded;
        item.candidate = Some(format!("# refreshed {}\n\n- summary", item.entity_name));
    }
    save_run(f, done, 1).await
}

#[tokio::test]
async fn reviews_approve_reject_and_write_back_candidates() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let gamma = f.entity(&kind, "gamma").await;
    let delta = f.entity(&kind, "delta").await;
    let policy = create_policy(&f, &kind, Some("source")).await;
    let policy_id = Uuid::parse_str(policy["id"].as_str().unwrap()).unwrap();
    let run = f
        .state
        .freshness("debug".into(), Op::RunCreate { policy_id })
        .await
        .unwrap()["item"]
        .clone();
    let run_id = run["id"].as_str().unwrap().to_string();
    let gamma_id = gamma["id"].as_str().unwrap().to_string();
    let delta_id = delta["id"].as_str().unwrap().to_string();
    let (status, body) = f
        .call(
            "POST",
            "/envs/debug/freshness/reviews",
            json!({"run_id": run_id, "entity_id": gamma_id, "verdict": "approve", "comment": "ok"}),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body}");

    let settled = succeed_all(&f, &run).await;
    assert_eq!(settled["status"], "succeeded");
    let approved = f
        .ok(
            "POST",
            "/envs/debug/freshness/reviews",
            json!({"run_id": run_id, "entity_id": gamma_id, "verdict": "approve", "comment": "fresh enough"}),
        )
        .await;
    assert_eq!(approved["entity"]["data_level"], "stable");
    assert_eq!(approved["entity"]["revision"], gamma["revision"].as_i64().unwrap() + 1);
    assert_eq!(approved["text_revision"]["revision"], 2, "candidate lands after the creation text");
    assert_eq!(approved["item"]["from_level"], "draft");

    let entities = f.ok("GET", "/envs/debug/entities", Value::Null).await;
    let row = entities["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"].as_str() == Some(gamma_id.as_str()))
        .unwrap();
    assert_eq!(row["data_level"], "stable");

    let detail = f.ok("GET", &format!("/envs/debug/entities/{gamma_id}"), Value::Null).await;
    let source = detail["text_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|attr| attr["definition"]["attribute_key"] == "source")
        .unwrap();
    let attribute: i64 = source["definition"]["id"].as_str().unwrap().parse().unwrap();
    let current = source["current"]["revision"].as_i64().unwrap();
    assert_eq!(current, 2);
    let content = f
        .ok("GET", &format!("/envs/debug/entities/{gamma_id}/attributes/{attribute}/text/{current}"), Value::Null)
        .await;
    assert!(content["content"].as_str().unwrap().contains("refreshed gamma"));

    let (status, body) = f
        .call(
            "POST",
            "/envs/debug/freshness/reviews",
            json!({"run_id": run_id, "entity_id": gamma_id, "verdict": "approve", "comment": "again"}),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body}");

    let rejected = f
        .ok(
            "POST",
            "/envs/debug/freshness/reviews",
            json!({"run_id": run_id, "entity_id": delta_id, "verdict": "reject", "comment": "stale candidate"}),
        )
        .await;
    assert_eq!(rejected["entity"]["data_level"], "draft");
    assert_eq!(rejected["entity"]["revision"], delta["revision"]);
    assert_eq!(rejected["item"]["from_level"], "draft");
    assert_eq!(rejected["item"]["to_level"], "draft");
    assert_eq!(rejected["text_revision"], Value::Null);

    let reviews = f
        .ok("GET", &format!("/envs/debug/freshness/reviews?run_id={run_id}"), Value::Null)
        .await;
    let rows = reviews["items"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row["verdict"] == "approve"));
    assert!(rows.iter().any(|row| row["verdict"] == "reject" && row["entity_id"].as_str() == Some(delta_id.as_str())));
    let all = f.ok("GET", "/envs/debug/freshness/reviews", Value::Null).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn viewer_role_reads_freshness_but_cannot_mutate() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let (status, _) = call(
        &f.router,
        Role::Viewer,
        "POST",
        "/envs/debug/freshness/policies",
        policy_body(&kind),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN);
    let (status, _) = call(&f.router, Role::Viewer, "GET", "/envs/debug/freshness/runs", Value::Null).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let (status, _) = call(&f.router, Role::Viewer, "GET", "/envs/debug/freshness/policies", Value::Null).await;
    assert_eq!(status, axum::http::StatusCode::OK);
}
