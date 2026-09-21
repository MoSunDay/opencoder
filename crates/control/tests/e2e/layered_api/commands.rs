//! Lifecycle commands: a layered run only accepts the three scheduler actions
//! and forwards them unchanged to the node that owns the projection.
use super::*;

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
