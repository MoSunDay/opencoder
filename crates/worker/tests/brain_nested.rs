#[path = "scheduler_v4/client.rs"]
mod client;
mod support;
use client::LayeredClient;
use serde_json::{json, Value};
use std::sync::Arc;
use support::Fleet;

fn plan(capability: &str) -> Value {
    json!({"schema_version":5,"title":"Nested plan","objective":"produce verified output",
        "nodes":[{"node_id":"work","title":"produce the assigned result","capability_ids":[capability],"layer":1,"objective":"produce verified output","success_criteria":"result verified"}],"edges":[]})
}

#[tokio::test]
async fn nested_plan_dispatches_a_real_child_and_reports_its_terminal_to_parent() {
    let model = Arc::new(LayeredClient::new());
    let fleet = Fleet::new(1, model.clone()).await;
    let saved=fleet.call("POST","/api/brain/plan-defs",json!({"id":"child-plan","version":1,"plan":plan("builtin-operator"),"changelog":"initial","created_at":1})).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"id":"brain-parent","schema_version":5,"plan":plan("plan-child-plan@1")}),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    let mut last = Value::Null;
    let view = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let view = fleet
                .call("GET", "/api/brain/runs/brain-parent/layered", Value::Null)
                .await;
            last = view.body.clone();
            if view.status == 200
                && ["completed", "failed", "blocked"]
                    .contains(&view.body["run"]["phase"].as_str().unwrap_or(""))
            {
                break view;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "nested parent did not settle: {last}; instructions: {:?}",
            model.instructions()
        )
    });
    assert_eq!(view.body["run"]["phase"], "completed", "{view:?}");
    let operation = &view.body["operations"][0];
    assert_eq!(operation["execution_kind"], "brain");
    assert_eq!(operation["status"], "done");
    let id = operation["execution_id"].as_str().unwrap();
    let child = fleet
        .call("GET", &format!("/api/brain/runs/{id}/layered"), Value::Null)
        .await;
    assert_eq!(child.body["run"]["phase"], "completed", "{child:?}");
    assert_eq!(child.body["run"]["parent"]["run_id"], "brain-parent");
    assert_eq!(child.body["operations"][0]["execution_kind"], "operator");
    assert_eq!(
        model.decisions(),
        4,
        "one dispatch and one closing decision per plan"
    );
    assert!(model
        .instructions()
        .iter()
        .any(|s| s.contains("Step task:\nproduce the assigned result")));
    fleet.shutdown().await;
}
