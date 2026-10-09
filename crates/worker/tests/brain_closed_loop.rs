#![cfg(not(windows))]
#[path = "brain_closed_loop/fixture.rs"]
mod fixture;
#[path = "brain_closed_loop/model.rs"]
mod model;
mod support;

use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc, time::Duration};

#[tokio::test]
async fn native_coding_test_loop_recovers_binding_errors_and_task_failures() {
    let model = Arc::new(model::Model::default());
    let fleet = support::Fleet::new(1, model.clone()).await;
    fixture::prepare(&fleet).await;
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({
                "id":"brain-native-loop","schema_version":7,"plan":fixture::plan()
            }),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    let mut last = Value::Null;
    let view = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let reply = fleet
                .call(
                    "GET",
                    "/api/brain/runs/brain-native-loop/layered",
                    Value::Null,
                )
                .await;
            assert_eq!(reply.status, 200, "{reply:?}");
            last = reply.body;
            assert!(
                !["blocked", "failed"].contains(&last["run"]["phase"].as_str().unwrap()),
                "{last}"
            );
            if last["run"]["phase"] == "completed" {
                break last.clone();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("loop did not finish: {last}"));
    assert_eq!(view["run"]["round"], 3);
    let operations = view["operations"].as_array().unwrap();
    assert_eq!(operations.len(), 5);
    assert_eq!(
        operations
            .iter()
            .filter(|op| op["status"] == "error")
            .count(),
        1
    );
    assert_eq!(
        operations
            .iter()
            .map(|op| op["execution_id"].as_str().unwrap())
            .collect::<HashSet<_>>()
            .len(),
        5
    );
    let mut outputs = Vec::new();
    for operation in operations.iter().filter(|op| op["status"] == "done") {
        let id = operation["execution_id"].as_str().unwrap();
        let detail = fleet
            .call("GET", &format!("/api/executions/{id}"), Value::Null)
            .await;
        assert_eq!(detail.status, 200, "{detail:?}");
        outputs.push((
            operation["activation"].as_u64().unwrap(),
            detail.body["result"]["scheduler_output"].clone(),
        ));
    }
    outputs.sort_by_key(|(activation, _)| *activation);
    let first_code = &outputs[0].1["code"];
    let first_test = &outputs[1].1["verify"];
    let fixed_code = &outputs[2].1["code"];
    let fixed_test = &outputs[3].1["verify"];
    assert_eq!(first_test["passed"], false);
    assert_eq!(fixed_test["passed"], true);
    assert_eq!(first_code["revision"], first_test["revision"]);
    assert_eq!(fixed_code["revision"], fixed_test["revision"]);
    assert_ne!(first_code["revision"], fixed_code["revision"]);
    assert!(fixed_code["source"].as_str().unwrap().contains("return 2"));
    assert_eq!(model.contexts.lock().unwrap().len(), 6);
    let events = view["events"].as_array().unwrap();
    assert!(events
        .iter()
        .any(|e| e["event_type"] == "operation_terminal"
            && e["reason_summary"]
                .as_str()
                .is_some_and(|text| text.contains("/code/absent"))));
    for visit in events
        .iter()
        .filter(|e| e["event_type"] == "layer_started" && e["activation"] != 1)
    {
        let prior = visit["activation"].as_u64().unwrap() - 1;
        assert!(events
            .iter()
            .any(|e| e["event_type"] == "layer_barrier_reached"
                && e["activation"] == prior
                && e["seq"].as_u64() < visit["seq"].as_u64()));
    }
    fleet.shutdown().await;
}
