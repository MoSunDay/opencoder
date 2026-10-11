use crate::support::Harness;
use reqwest::Method;
use serde_json::json;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

#[tokio::test]
async fn conversation_lists_wait_for_remote_durable_acceptance() {
    for kind in ["operator", "agent"] {
        let h = Harness::new().await;
        let id = format!("{kind}-delayed-acceptance");
        let gate = Arc::new(tokio::sync::Notify::new());
        *h.node.create_gate.lock().unwrap() = Some((id.clone(), gate.clone()));
        let body = json!({"id": id, "kind": kind, "agent":"act", "prompt":"once"});
        let submit = {
            let h = h.clone();
            let body = body.clone();
            tokio::spawn(async move { h.req(Method::POST, "/api/sessions", Some(body)).await })
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            while h.node.gated_creates.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(h.state.fleet.assignment(&id).await.unwrap().is_some());
        assert!(h.state.fleet.index(&id).await.unwrap().is_none());
        for (path, field) in [
            (format!("/api/executions?kind={kind}"), "executions"),
            (format!("/api/sessions?kind={kind}"), "sessions"),
            (
                format!("/api/nodes/node-e2e/dialogs?kind={kind}"),
                "dialogs",
            ),
        ] {
            let (status, result) = h.req(Method::GET, &path, None).await;
            assert_eq!(status, 200, "{result}");
            assert!(result[field].as_array().unwrap().is_empty(), "{result}");
        }
        gate.notify_one();
        let (status, accepted) = submit.await.unwrap();
        assert_eq!(status, 200, "{accepted}");
        assert_eq!(accepted["id"], id);
        let (status, repeated) = h.req(Method::POST, "/api/sessions", Some(body)).await;
        assert_eq!(status, 200, "{repeated}");
        assert_eq!(accepted, repeated);
        let (_, page) = h.req(Method::GET, "/api/executions", None).await;
        assert_eq!(page["executions"].as_array().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn rejected_conversation_does_not_leave_a_phantom_execution() {
    for kind in ["operator", "agent"] {
        let h = Harness::new().await;
        h.node
            .set_create_reply(400, json!({"error":"selected identity resource missing"}));
        let id = format!("{kind}-rejected-acceptance");
        let body = json!({"id":id,"kind":kind,"prompt":"once"});
        for _ in 0..2 {
            let (status, result) = h
                .req(Method::POST, "/api/sessions", Some(body.clone()))
                .await;
            assert_eq!(status, 400, "{result}");
            assert_eq!(result["error"], "selected identity resource missing");
            assert!(h.state.fleet.index(&id).await.unwrap().is_none());
        }
        let (_, result) = h.req(Method::GET, "/api/executions", None).await;
        assert!(result["executions"].as_array().unwrap().is_empty());
    }
}
