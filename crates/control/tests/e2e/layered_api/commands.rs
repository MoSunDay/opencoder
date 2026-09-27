//! Lifecycle commands: a layered run only accepts the three scheduler actions
//! and forwards them unchanged to the node that owns the projection.
use super::*;

#[tokio::test]
async fn human_input_is_recorded_only_as_a_brain_event() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    h.node.set_brain("snapshot", 200, snapshot("waiting", 1, 3));
    h.node
        .set_brain("human_input", 200, json!({"run":{"phase":"ready"}}));
    let path = format!("/api/brain/runs/{RUN}/inputs");
    let (status, body) = h
        .req(
            Method::POST,
            &path,
            Some(json!({"text":"  review evidence  "})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["delivery"], json!("brain_event"));
    let calls = h.node.brain_calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.action == "human_input")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .find(|call| call.action == "human_input")
            .unwrap()
            .input,
        json!({"text":"review evidence"})
    );
    assert!(calls.iter().all(|call| call.action != "steer"));

    let (status, _) = h
        .req(
            Method::POST,
            &path,
            Some(json!({"text":"x","execution_id":"agent-1"})),
        )
        .await;
    assert_eq!(status, 422);
    let (status, _) = h
        .req(Method::POST, &path, Some(json!({"text":"   "})))
        .await;
    assert_eq!(status, 400);
    h.node
        .set_brain("snapshot", 200, snapshot("completed", 1, 4));
    let (status, _) = h
        .req(Method::POST, &path, Some(json!({"text":"too late"})))
        .await;
    assert_eq!(status, 409);
    assert_eq!(
        h.node
            .brain_calls()
            .iter()
            .filter(|call| call.action == "human_input")
            .count(),
        1
    );
}

#[tokio::test]
async fn layered_commands_forward_only_the_three_lifecycle_actions() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    create(&h).await;
    h.node
        .set_brain("pause", 200, json!({"phase":"paused","generation":4}));
    let path = format!("/api/brain/runs/{RUN}/commands");
    let (status, body) = h
        .req(Method::POST, &path, Some(json!({"action":"pause"})))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["phase"], json!("paused"));
    let (status, body) = h
        .req(Method::POST, &path, Some(json!({"action":"restart"})))
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("supported commands"), "{body}");

    let calls = h.node.brain_calls();
    let pause = calls.iter().find(|call| call.action == "pause").unwrap();
    assert_eq!(pause.execution.id, RUN);
    assert_eq!(pause.input, json!(null));

    // Unknown and historical runs are never layered command targets.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/runs/brain-missing/commands",
            Some(json!({"action":"pause"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert!(body.to_string().contains("migration required"), "{body}");
}
