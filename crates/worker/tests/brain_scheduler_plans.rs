#[path = "support/scheduler.rs"]
mod scheduler_support;
#[path = "support/mod.rs"]
mod support;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;

fn plan() -> Value {
    json!({"id":"plan-simple","version":1,"changelog":"initial","created_at":1,
        "plan":{"schema_version":3,"title":"Reusable review","objective":"inspect repository",
        "inputs":{"repo":"default"},"capability_ids":["builtin-agent-act"],"max_rounds":3}})
}

#[tokio::test]
async fn saved_plan_runs_resolve_scope_inputs_and_persist_round_evidence() {
    let (_config, _home) = isolated_config();
    let fleet = Fleet::new(1, Arc::new(scheduler_support::SchedulerClient::default())).await;
    let saved = fleet.call("POST", "/api/brain/plan-defs", plan()).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let retry = fleet.call("POST", "/api/brain/plan-defs", plan()).await;
    assert_eq!(retry.status, 200, "{retry:?}");
    let mut second = plan();
    second["plan"]["title"] = json!("New title");
    assert_eq!(
        fleet
            .call("POST", "/api/brain/plan-defs", second.clone())
            .await
            .status,
        409
    );
    second["version"] = json!(2);
    assert_eq!(
        fleet
            .call("POST", "/api/brain/plan-defs", second)
            .await
            .status,
        200
    );
    let list = fleet.call("GET", "/api/brain/plan-defs", Value::Null).await;
    assert_eq!(list.body["plans"][0]["schema_version"], 3);
    let request = json!({"id":"brain-saved-plan","schema_version":3,"plan":{"id":"plan-simple","version":1},"inputs":{"repo":"overridden"}});
    let created = fleet.call("POST", "/api/brain/runs", request.clone()).await;
    assert_eq!(created.status, 202, "{created:?}");
    assert_eq!(created.body["run_id"], "brain-saved-plan");
    scheduler_support::wait_phase_within(&fleet, "brain-saved-plan", "completed", 90).await;
    let view = fleet
        .call("GET", "/api/brain/runs/brain-saved-plan/view", Value::Null)
        .await;
    assert_eq!(view.status, 200, "{view:?}");
    assert_eq!(view.body["plan"], json!({"id":"plan-simple","version":1}));
    assert_eq!(view.body["capability_ids"], json!(["builtin-agent-act"]));
    assert_eq!(view.body["capabilities"].as_array().unwrap().len(), 1);
    let operation = &view.body["rounds"][0]["operations"][0];
    assert_eq!(operation["execution_created"], true);
    assert_eq!(operation["execution_kind"], "agent");
    assert_eq!(operation["capability"]["target"], "act");
    assert!(view.body["rounds"][0]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["reason_summary"] == "inspect repository"));
    let round = fleet
        .call(
            "GET",
            "/api/brain/runs/brain-saved-plan/rounds/1",
            Value::Null,
        )
        .await;
    assert_eq!(round.body, view.body["rounds"][0]);
    let detail = fleet
        .call(
            "GET",
            &format!(
                "/api/executions/{}",
                operation["execution_id"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(detail.status, 200, "{detail:?}");
    let assignment = fleet
        .state
        .fleet
        .assignment(operation["execution_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        assignment.request.input["scheduler_inputs"]["request"],
        "overridden"
    );
    assert_eq!(
        fleet.call("POST", "/api/brain/runs", request).await.status,
        202
    );
    let mut another = json!({"id":"brain-saved-another","schema_version":3,"plan":{"id":"plan-simple","version":1}});
    assert_eq!(
        fleet
            .call("POST", "/api/brain/runs", another.clone())
            .await
            .status,
        202
    );
    another["plan"]["version"] = json!(2);
    assert_eq!(
        fleet.call("POST", "/api/brain/runs", another).await.status,
        409
    );
    scheduler_support::wait_phase_within(&fleet, "brain-saved-another", "completed", 90).await;
    fleet.shutdown().await;
}

#[tokio::test]
async fn missing_or_empty_plan_scope_is_rejected_before_any_execution() {
    let (_config, _home) = isolated_config();
    let fleet = Fleet::new(1, mock()).await;
    for ids in [
        json!([]),
        json!(["missing"]),
        json!(["builtin-agent-act", "builtin-agent-act"]),
    ] {
        let mut version = plan();
        version["plan"]["capability_ids"] = ids;
        let reply = fleet.call("POST", "/api/brain/plan-defs", version).await;
        assert_eq!(reply.status, 400, "{reply:?}");
    }
    let reply = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"schema_version":3,"objective":"test","capability_ids":["missing"]}),
        )
        .await;
    assert_eq!(reply.status, 400, "{reply:?}");
    assert!(fleet
        .state
        .fleet
        .indexes(None, None, 100)
        .await
        .unwrap()
        .is_empty());
    fleet.shutdown().await;
}

#[path = "scheduler_browser/fixture.rs"]
mod browser_fixture;

#[tokio::test]
async fn five_types_dispatch_in_two_rounds_and_retain_original_execution_metadata() {
    let fleet = Fleet::new(1, Arc::new(browser_fixture::BrowserClient)).await;
    browser_fixture::seed(&fleet).await;
    let mut body = plan();
    body["plan"]["capability_ids"] = json!([
        "browser-agent",
        "browser-team",
        "browser-dag",
        "browser-todos",
        "browser-operator"
    ]);
    body["plan"]["inputs"] = json!({});
    assert_eq!(
        fleet
            .call("POST", "/api/brain/plan-defs", body)
            .await
            .status,
        200
    );
    let run = fleet.call("POST", "/api/brain/runs", json!({"id":"brain-five-types","schema_version":3,"plan":{"id":"plan-simple","version":1}})).await;
    assert_eq!(run.status, 202, "{run:?}");
    scheduler_support::wait_phase_within(&fleet, "brain-five-types", "completed", 120).await;
    let view = fleet
        .call("GET", "/api/brain/runs/brain-five-types/view", Value::Null)
        .await;
    assert_eq!(view.status, 200, "{view:?}");
    assert_eq!(view.body["rounds"].as_array().unwrap().len(), 2);
    let mut kinds = vec![];
    for round in view.body["rounds"].as_array().unwrap() {
        assert!(!round["decisions"].as_array().unwrap().is_empty());
        for op in round["operations"].as_array().unwrap() {
            kinds.push(op["execution_kind"].as_str().unwrap());
            assert_eq!(op["status"], "done");
            assert_eq!(op["execution_created"], true);
            let detail = fleet
                .call(
                    "GET",
                    &format!("/api/executions/{}", op["execution_id"].as_str().unwrap()),
                    Value::Null,
                )
                .await;
            assert_eq!(detail.status, 200, "{detail:?}");
            assert_eq!(detail.body["execution"]["kind"], op["execution_kind"]);
        }
    }
    kinds.sort();
    assert_eq!(kinds, ["agent", "dag", "operator", "team", "todos"]);
    fleet
        .state
        .fleet
        .put_definition(
            "brain_capability",
            "browser-agent",
            &json!({"id":"browser-agent","target":"changed"}),
        )
        .await
        .unwrap();
    let history = fleet
        .call("GET", "/api/brain/runs/brain-five-types/view", Value::Null)
        .await;
    assert_eq!(
        history.body, view.body,
        "historical metadata must not depend on the current catalog"
    );
    fleet.shutdown().await;
}

#[tokio::test]
async fn registered_team_capability_uses_real_definition_and_reports_missing_targets() {
    let fleet = Fleet::new(1, mock()).await;
    let team = json!({"name":"registered-team","captain":"act","members":[{"agent":"act"},{"agent":"plan"}]});
    assert_eq!(fleet.call("POST", "/api/teams", team).await.status, 200);
    let created = fleet.call("POST", "/api/brain/capabilities", json!({"capability_type":"team","summary":"Review with team","input_desc":"request","output_desc":"evidence","eng_inputs":[]})).await;
    assert_eq!(created.status, 201, "{created:?}");
    let id = created.body["capability"]["id"].as_str().unwrap();
    let path = format!("/api/brain/capabilities/{id}/target");
    assert_eq!(
        fleet
            .call(
                "PUT",
                &path,
                json!({"kind":"team","target":"registered-team"})
            )
            .await
            .status,
        200
    );
    let library = fleet.call("GET", "/api/brain/library", Value::Null).await;
    assert_eq!(library.status, 200, "{library:?}");
    let capability = library.body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert_eq!(capability["definition"]["captain"], "act");
    assert_eq!(
        capability["definition"]["members"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        fleet
            .call("PUT", &path, json!({"kind":"team","target":"gone"}))
            .await
            .status,
        200
    );
    let library = fleet.call("GET", "/api/brain/library", Value::Null).await;
    let capability = library.body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap();
    assert!(capability["definition"].is_null());
    assert!(capability["unavailable_reason"]
        .as_str()
        .unwrap()
        .contains("not found"));
    let mut body = plan();
    body["plan"]["capability_ids"] = json!([id]);
    assert_eq!(
        fleet
            .call("POST", "/api/brain/plan-defs", body)
            .await
            .status,
        400
    );
    fleet.shutdown().await;
}
