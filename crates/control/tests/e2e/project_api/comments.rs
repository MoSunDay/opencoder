use crate::support::Harness;
use reqwest::Method;
use serde_json::{json, Value};

async fn todo(h: &Harness) -> String {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"notes","draft":"body"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn comments_use_authenticated_users_and_enforce_permissions_validation_and_idempotency() {
    let h = Harness::new().await;
    let id = todo(&h).await;
    let path = format!("/api/project/todos/{id}/comments");
    let (_, user) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"alice","role":"editor"})),
        )
        .await;
    let token = user["token"].as_str().unwrap();
    let body = json!({"request_id":"note-1","body_md":"  **keep indentation**\n"});
    let response = h
        .req_raw(Method::POST, &path, Some(body.clone()), Some(token))
        .await;
    assert_eq!(response.status(), 200);
    let first: Value = response.json().await.unwrap();
    assert_eq!(first["author_id"], "alice");
    assert_eq!(first["submitted_by"], "alice");
    assert_eq!(first["body_md"], body["body_md"]);
    let retry: Value = h
        .req_raw(Method::POST, &path, Some(body), Some(token))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(retry["seq"], first["seq"]);
    assert_eq!(
        h.req_raw(
            Method::POST,
            &path,
            Some(json!({"request_id":"note-1","body_md":"changed"})),
            Some(token)
        )
        .await
        .status(),
        409
    );
    for body in [
        json!({"request_id":"bad","body_md":" "}),
        json!({"request_id":"bad","body_md":"x".repeat(65537)}),
        json!({"request_id":"bad","body_md":"body","author_id":"bob"}),
    ] {
        let expected = if body.get("author_id").is_some() {
            422
        } else {
            400
        };
        assert_eq!(h.req(Method::POST, &path, Some(body)).await.0, expected);
    }
    let (_, user) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"bob","role":"viewer"})),
        )
        .await;
    let token = user["token"].as_str().unwrap();
    assert_eq!(
        h.req_raw(Method::GET, &path, None, Some(token))
            .await
            .status(),
        200
    );
    assert_eq!(
        h.req_raw(
            Method::POST,
            &path,
            Some(json!({"request_id":"viewer","body_md":"body"})),
            Some(token)
        )
        .await
        .status(),
        403
    );
    assert_eq!(
        h.req_raw(Method::GET, &path, None, None).await.status(),
        401
    );
    assert_eq!(
        h.req(
            Method::GET,
            &(path.clone() + "?before_seq=1&after_seq=2"),
            None
        )
        .await
        .0,
        400
    );
    assert_eq!(
        h.req(Method::GET, &(path + "?limit=101"), None).await.0,
        400
    );
    assert_eq!(
        h.req(Method::GET, "/api/project/todos/missing/comments", None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn all_six_capability_signatures_keep_snapshots_and_retries_survive_registry_removal() {
    let h = Harness::new().await;
    let path = format!("/api/project/todos/{}/comments", todo(&h).await);
    for kind in ["agent", "operator", "team", "dag", "todos", "brain"] {
        let cap = format!("note-{kind}");
        h.state
            .fleet
            .put_definition(
                "brain_capability",
                &cap,
                &json!({"id":cap,"kind":kind,"target":"original name","definition":{}}),
            )
            .await
            .unwrap();
        let body = json!({"request_id":cap,"body_md":"**capability note**","capability_id":cap});
        let (status, comment) = h.req(Method::POST, &path, Some(body.clone())).await;
        assert_eq!(status, 200, "{comment}");
        assert_eq!(comment["author_kind"], "capability");
        assert_eq!(comment["capability_kind"], kind);
        assert_eq!(comment["submitted_by"], "admin");
        h.state
            .fleet
            .delete_definition("brain_capability", &cap)
            .await
            .unwrap();
        let (status, retry) = h.req(Method::POST, &path, Some(body)).await;
        assert_eq!(status, 200, "{retry}");
        assert_eq!(retry["seq"], comment["seq"]);
        assert_eq!(retry["author_name"], "original name");
    }
    assert_eq!(
        h.req(
            Method::POST,
            &path,
            Some(json!({"request_id":"missing","body_md":"note","capability_id":"missing"}))
        )
        .await
        .0,
        404
    );
    let (_, page) = h.req(Method::GET, &path, None).await;
    assert_eq!(page["comments"].as_array().unwrap().len(), 6);
}

#[tokio::test]
async fn comment_execution_sources_match_admission_and_survive_unlinking() {
    let h = Harness::new().await;
    let id = todo(&h).await;
    let root = format!("/api/project/todos/{id}");
    let (status, reply) = h.dispatch(Method::POST, &format!("{root}/dispatch"), Some(json!({"execution_id":"agent-note-source","capability_id":"builtin-agent-act","input":{"prompt":"work"}}))).await;
    assert_eq!(status, 202, "{reply}");
    let body = json!({"request_id":"source","body_md":"source note","capability_id":"builtin-agent-act","execution_id":"agent-note-source"});
    let path = format!("{root}/comments");
    let (status, comment) = h.req(Method::POST, &path, Some(body.clone())).await;
    assert_eq!(status, 200, "{comment}");
    assert_eq!(h.req(Method::POST, &path, Some(json!({"request_id":"wrong","body_md":"wrong","capability_id":"builtin-operator","execution_id":"agent-note-source"}))).await.0, 409);
    h.req(
        Method::DELETE,
        &format!("{root}/executions/agent-note-source"),
        None,
    )
    .await;
    assert_eq!(h.req(Method::POST, &path, Some(body)).await.0, 200);
    assert_eq!(h.req(Method::POST, &path, Some(json!({"request_id":"unlinked","body_md":"wrong","execution_id":"agent-note-source"}))).await.0, 409);
    h.req(Method::DELETE, &root, None).await;
    assert_eq!(h.req(Method::GET, &path, None).await.0, 404);
    assert!(h
        .state
        .fleet
        .index("agent-note-source")
        .await
        .unwrap()
        .is_some());
}
