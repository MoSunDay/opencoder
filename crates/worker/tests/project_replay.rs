mod support;
use base64::Engine;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;

async fn read(fleet: &Fleet, path: &str) -> Value {
    let reply = fleet.call("GET", path, Value::Null).await;
    assert_eq!(reply.status, 200, "{path}: {reply:?}");
    reply.body
}
async fn wait_index(fleet: &Fleet, id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if fleet.state.fleet.index(id).await.unwrap().is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
async fn field(fleet: &Fleet, id: &str, name: &str) -> String {
    let mut bytes = Vec::new();
    loop {
        let page = read(
            fleet,
            &format!(
                "/api/executions/{id}/detail-field?field={name}&offset={}",
                bytes.len()
            ),
        )
        .await;
        let part = base64::engine::general_purpose::STANDARD
            .decode(page["bytes_b64"].as_str().unwrap())
            .unwrap();
        assert!(part.len() <= 65536);
        bytes.extend(part);
        if page["eof"] == true {
            break;
        }
    }
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn project_indexes_replay_all_attempts_with_pagination_and_large_fields() {
    let fleet = Fleet::new(2, mock()).await;
    let owner = fleet.nodes[0].registration().id;
    let goal = fleet
        .call(
            "POST",
            "/api/project/goals",
            json!({"title":"replay project"}),
        )
        .await;
    assert_eq!(goal.status, 200);
    let milestone = fleet
        .call(
            "POST",
            "/api/project/milestones",
            json!({"title":"replay milestone","goal_id":goal.body["id"]}),
        )
        .await;
    assert_eq!(milestone.status, 200);
    let draft = "large input 界".repeat(6000);
    let todo = fleet.call("POST", "/api/project/todos", json!({"title":"trace every attempt","draft":draft,"milestone_id":milestone.body["id"],"agent":"act"})).await;
    assert_eq!(todo.status, 200);
    let todo = todo.body["id"].as_str().unwrap();
    let root = format!("project-{todo}");
    let mut ids = Vec::new();
    for version in 1..=27 {
        let action = if version == 1 { "plan" } else { "execute" };
        let id = format!("prun-replay-{version}");
        let route = format!("/api/project/todos/{todo}/{action}");
        let input = json!({"run_id":id,"node_id":owner});
        let receipt = fleet.call("POST", &route, input.clone()).await;
        assert!([200, 202].contains(&receipt.status), "{receipt:?}");
        assert_eq!(receipt.body["run_id"], id);
        assert_eq!(receipt.body["node_id"], owner);
        let done = settled(&fleet.nodes[0], &root).await;
        assert_eq!(done["execution"]["status"], "idle", "{done}");
        let repeated = fleet.call("POST", &route, input).await;
        assert!([200, 202].contains(&repeated.status), "{repeated:?}");
        assert_eq!(repeated.body["run_id"], id);
        wait_index(&fleet, &id).await;
        let detail = read(&fleet, &format!("/api/executions/{id}")).await;
        assert_eq!(detail["retention"], "complete", "{detail}");
        assert_eq!(detail["run"]["version"], version);
        assert_eq!(detail["run"]["input_snapshot"]["omitted"], true);
        let input: Value = serde_json::from_str(
            &field(&fleet, &id, &format!("project.run.{id}.input_snapshot")).await,
        )
        .unwrap();
        assert_eq!(input["todo"]["draft"], draft);
        assert_eq!(
            input["agent"]["name"],
            if version == 1 { "plan" } else { "act" }
        );
        let wire: Value =
            serde_json::from_str(&field(&fleet, &id, "archive.request-1.json").await).unwrap();
        assert!(wire["messages"].is_array());
        if version == 1 {
            assert!(wire.to_string().contains("large input"));
        }
        let messages = read(&fleet, &format!("/api/executions/{id}/messages")).await;
        assert!(!messages["chunks"].as_array().unwrap().is_empty());
        assert_eq!(
            messages["chunks"][0]["role"], "user",
            "first input was skipped"
        );
        assert_eq!(messages["more"], false);
        let mut after = 0;
        loop {
            let events = read(
                &fleet,
                &format!("/api/executions/{id}/events-page?after={after}"),
            )
            .await;
            for event in events["events"].as_array().unwrap() {
                assert_eq!(event["seq"], after + 1);
                after += 1;
            }
            if events["more"] == false {
                break;
            }
        }
        assert_eq!(detail["replay"]["event_count"], after);
        ids.push((id, detail["replay"].clone()));
    }
    let page = read(&fleet, &format!("/api/project/todos/{todo}/runs")).await;
    assert_eq!(page["runs"].as_array().unwrap().len(), 20);
    assert_eq!(page["more"], true);
    let next = read(
        &fleet,
        &format!(
            "/api/project/todos/{todo}/runs?before_version={}",
            page["next_version"]
        ),
    )
    .await;
    assert_eq!(next["runs"].as_array().unwrap().len(), 7);
    assert_eq!(next["more"], false);
    for (id, original) in &ids {
        let detail = read(&fleet, &format!("/api/executions/{id}")).await;
        assert_eq!(&detail["replay"], original, "old run boundaries changed");
    }
    let beyond = read(
        &fleet,
        &format!(
            "/api/executions/{}/events-page?after={}",
            ids[0].0,
            i64::MAX
        ),
    )
    .await;
    assert_eq!(beyond["events"], json!([]));
    assert_eq!(beyond["more"], false);
    for pair in ids[1..].windows(2) {
        assert_eq!(pair[0].1["messages_through"], pair[1].1["messages_after"]);
    }
    let original = read(&fleet, &format!("/api/executions/{}/messages", ids[1].0)).await;
    let end = ids[1].1["messages_through"].as_i64().unwrap();
    assert!(original["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|chunk| chunk["seq"].as_i64().unwrap() <= end));
    let other = fleet.nodes[1].indexes().await.unwrap();
    assert!(!other
        .iter()
        .any(|index| index.id == root || index.id.starts_with("prun-replay-")));
    fleet.disconnect(0).await;
    assert_eq!(
        fleet
            .call("GET", &format!("/api/executions/{}", ids[0].0), Value::Null)
            .await
            .status,
        503
    );
    fleet.shutdown().await;
}

#[tokio::test]
async fn a_rejected_first_execute_does_not_prevent_later_planning() {
    let fleet = Fleet::new(1, mock()).await;
    let todo = fleet
        .call(
            "POST",
            "/api/project/todos",
            json!({"title":"plan after rejection","draft":"create plan first","agent":"act"}),
        )
        .await;
    assert_eq!(todo.status, 200);
    let id = todo.body["id"].as_str().unwrap();
    let execute = fleet
        .call(
            "POST",
            &format!("/api/project/todos/{id}/execute"),
            json!({"run_id":"prun-rejected"}),
        )
        .await;
    assert_eq!(execute.status, 409, "{execute:?}");
    let plan = fleet
        .call(
            "POST",
            &format!("/api/project/todos/{id}/plan"),
            json!({"run_id":"prun-after-rejection"}),
        )
        .await;
    assert_eq!(plan.status, 202, "{plan:?}");
    let done = settled(&fleet.nodes[0], &format!("project-{id}")).await;
    assert_eq!(done["execution"]["status"], "idle");
    let runs = read(&fleet, &format!("/api/project/todos/{id}/runs")).await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 1);
    assert_eq!(runs["runs"][0]["id"], "prun-after-rejection");
    fleet.shutdown().await;
}

#[tokio::test]
async fn failed_attempt_keeps_its_only_input_message_after_later_runs() {
    let client = mock();
    let fleet = Fleet::new(1, client.clone()).await;
    let todo = fleet
        .call(
            "POST",
            "/api/project/todos",
            json!({
                "title":"retain failed input", "draft":"input stays available", "agent":"act"
            }),
        )
        .await;
    assert_eq!(todo.status, 200);
    let todo = todo.body["id"].as_str().unwrap();
    let root = format!("project-{todo}");
    for (action, id, failed) in [
        ("plan", "prun-message-plan", false),
        ("execute", "prun-message-before", false),
        ("execute", "prun-message-failed", true),
        ("execute", "prun-message-after", false),
    ] {
        if failed {
            client.queue_script(vec![opencoder_llm::LlmEvent::Error(
                "injected failure".into(),
            )]);
        }
        let started = fleet
            .call(
                "POST",
                &format!("/api/project/todos/{todo}/{action}"),
                json!({"run_id":id}),
            )
            .await;
        assert!([200, 202].contains(&started.status), "{started:?}");
        let result = settled(&fleet.nodes[0], &root).await;
        assert_eq!(
            result["execution"]["status"],
            if failed { "error" } else { "idle" }
        );
    }
    let id = "prun-message-failed";
    wait_index(&fleet, id).await;
    let detail = read(&fleet, &format!("/api/executions/{id}")).await;
    assert_eq!(detail["run"]["status"], "failed");
    let page = read(&fleet, &format!("/api/executions/{id}/messages")).await;
    assert_eq!(page["chunks"].as_array().unwrap().len(), 1);
    assert_eq!(page["chunks"][0]["role"], "user");
    assert_eq!(
        page["chunks"][0]["seq"],
        detail["replay"]["messages_through"]
    );
    assert_eq!(page["more"], false);
    let rebased = read(
        &fleet,
        &format!(
            "/api/executions/{id}/messages?seq={}&offset=1",
            detail["replay"]["messages_after"]
        ),
    )
    .await;
    assert_eq!(
        rebased, page,
        "an earlier offset cannot skip the first input"
    );
    fleet.shutdown().await;
}
