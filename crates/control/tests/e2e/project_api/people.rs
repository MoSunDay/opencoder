use crate::support::{Harness, TOKEN};
use reqwest::Method;
use serde_json::{json, Value};

#[tokio::test]
async fn project_people_crud_directory_permissions_and_atomic_validation() {
    let h = Harness::new().await;
    let mut viewer = String::new();
    for (name, role) in [("alice", "editor"), ("bob", "viewer")] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/users",
                Some(json!({"name":name,"role":role})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        if name == "bob" {
            viewer = body["token"].as_str().unwrap().into();
        }
    }
    let response = h
        .req_raw(Method::GET, "/api/project/people", None, Some(&viewer))
        .await;
    assert_eq!(response.status(), 200);
    let people: Value = response.json().await.unwrap();
    assert!(people["people"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == &json!({"name":"alice"})));
    assert!(people["people"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p.as_object().unwrap().len() == 1));
    assert_eq!(
        h.req_raw(Method::GET, "/api/users", None, Some(&viewer))
            .await
            .status(),
        403
    );
    for (collection, extra) in [
        ("goals", json!({})),
        ("initiatives", json!({})),
        ("todos", json!({"draft":"task"})),
    ] {
        let path = format!("/api/project/{collection}");
        let mut body = json!({"title":"original","owner_user_name":"alice","follower_user_names":["bob","alice","bob"]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (status, created) = h.req(Method::POST, &path, Some(body)).await;
        assert_eq!(status, 200, "{created}");
        assert_eq!(created["follower_user_names"], json!(["alice", "bob"]));
        let path = format!("{path}/{}", created["id"].as_str().unwrap());
        let change = json!({"title":"invalid","owner_user_name":"missing"});
        assert_eq!(h.req(Method::PATCH, &path, Some(change)).await.0, 400);
        let (_, current) = h.req(Method::GET, &path, None).await;
        assert_eq!(current["title"], "original");
        assert_eq!(current["owner_user_name"], "alice");
        assert_eq!(
            h.req_raw(
                Method::PATCH,
                &path,
                Some(json!({"owner_user_name":"bob"})),
                Some(&viewer)
            )
            .await
            .status(),
            403
        );
        assert_eq!(
            h.req(Method::PATCH, &path, Some(json!({"owner_user_name":null})))
                .await
                .0,
            200
        );
        let (_, current) = h.req(Method::GET, &path, None).await;
        assert!(current["owner_user_name"].is_null());
        assert_eq!(current["follower_user_names"], json!(["alice", "bob"]));
    }
    assert_eq!(
        h.req_raw(Method::GET, "/api/project/people", None, Some(TOKEN))
            .await
            .status(),
        200
    );
}
