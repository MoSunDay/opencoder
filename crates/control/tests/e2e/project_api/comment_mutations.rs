use crate::support::Harness;
use reqwest::Method;
use serde_json::{json, Value};

#[tokio::test]
async fn comment_edit_delete_routes_enforce_ownership_revisions_and_validation() {
    let h = Harness::new().await;
    let (_, task) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"Task","draft":"body"})),
        )
        .await;
    let root = format!(
        "/api/project/todos/{}/comments",
        task["id"].as_str().unwrap()
    );
    let (_, user) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"alice","role":"editor"})),
        )
        .await;
    let token = user["token"].as_str().unwrap();
    let note = json!({"request_id":"note","body_md":"original"});
    let response = h
        .req_raw(Method::POST, &root, Some(note.clone()), Some(token))
        .await;
    assert_eq!(response.status(), 200);
    let comment: Value = response.json().await.unwrap();
    let path = format!("{root}/{}", comment["seq"]);
    let input = json!({"body_md":"merged","expected_revision":1});
    let (_, other) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"bob","role":"editor"})),
        )
        .await;
    let (_, viewer) = h
        .req(
            Method::POST,
            "/api/users",
            Some(json!({"name":"reader","role":"viewer"})),
        )
        .await;
    for user in [&other, &viewer] {
        let token = user["token"].as_str().unwrap();
        assert_eq!(
            h.req_raw(Method::PATCH, &path, Some(input.clone()), Some(token))
                .await
                .status(),
            403
        );
        assert_eq!(
            h.req_raw(
                Method::DELETE,
                &format!("{path}?expected_revision=1"),
                None,
                Some(token)
            )
            .await
            .status(),
            403
        );
    }
    for (body, expected) in [
        (json!({"body_md":" ","expected_revision":1}), 400),
        (
            json!({"body_md":"x".repeat(65537),"expected_revision":1}),
            400,
        ),
        (json!({"body_md":"x","expected_revision":0}), 400),
        (
            json!({"body_md":"x","expected_revision":1,"author_id":"forged"}),
            422,
        ),
    ] {
        assert_eq!(h.req(Method::PATCH, &path, Some(body)).await.0, expected);
    }
    let updated = h
        .req_raw(Method::PATCH, &path, Some(input.clone()), Some(token))
        .await;
    assert_eq!(updated.status(), 200);
    let updated: Value = updated.json().await.unwrap();
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["author_name"], "alice");
    assert_eq!(updated["created_at"], comment["created_at"]);
    assert_eq!(h.req(Method::PATCH, &path, Some(input)).await.0, 409);
    assert_eq!(
        h.req(Method::DELETE, &format!("{path}?expected_revision=1"), None)
            .await
            .0,
        409
    );
    let retry: Value = h
        .req_raw(Method::POST, &root, Some(note.clone()), Some(token))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(retry["body_md"], "merged");
    let delete = format!("{path}?expected_revision=2");
    assert_eq!(h.req(Method::DELETE, &delete, None).await.0, 200);
    assert_eq!(h.req(Method::DELETE, &delete, None).await.0, 200);
    assert_eq!(
        h.req(Method::GET, &root, None).await.1["comments"],
        json!([])
    );
    assert_eq!(
        h.req_raw(Method::POST, &root, Some(note), Some(token))
            .await
            .status(),
        409
    );
    assert_eq!(
        h.req(
            Method::PATCH,
            &path,
            Some(json!({"body_md":"x","expected_revision":2}))
        )
        .await
        .0,
        404
    );
}
