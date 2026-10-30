mod common;
use axum::http::StatusCode;
use common::Fixture;
use serde_json::{json, Value};

#[tokio::test]
async fn entity_mounts_multiple_directories_but_duplicate_membership_conflicts() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let root = f
        .ok("GET", "/envs/debug/directories/tree", Value::Null)
        .await["root_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 A","parent_id":root}),
        )
        .await["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let b = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 B","parent_id":root}),
        )
        .await["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = f.entity(&kind, "多目录实体").await;
    let e = e["id"].as_str().unwrap();
    let membership = f
        .ok("GET", "/envs/debug/relationship-types", Value::Null)
        .await["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["is_directory_membership"] == true)
        .map(|item| item["id"].clone())
        .unwrap();
    let mount = |target: String| {
        json!({"relationship_type_id":membership,"source_entity_id":e,"target_entity_id":target})
    };
    f.ok("POST", "/envs/debug/relationships", mount(a.clone())).await;
    f.ok("POST", "/envs/debug/relationships", mount(b.clone())).await;
    assert_eq!(
        f.call("POST", "/envs/debug/relationships", mount(a.clone()))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let anchored = f
        .ok(
            "GET",
            &format!("/envs/debug/relationships?anchor={e}"),
            Value::Null,
        )
        .await;
    let targets = anchored["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| {
            item["relationship_type_id"] == membership && item["source_entity_id"] == json!(e)
        })
        .filter(|item| !item["is_deleted"].as_bool().unwrap_or(true))
        .map(|item| item["target_entity_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 2, "{anchored:?}");
    for directory in [&a, &b] {
        assert!(targets.contains(&directory.as_str()), "{anchored:?}");
    }
}

#[tokio::test]
async fn directory_nodes_keep_single_parent_and_tree_invariants() {
    let f = Fixture::new().await;
    let root = f
        .ok("GET", "/envs/debug/directories/tree", Value::Null)
        .await["root_id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 A","parent_id":root}),
        )
        .await["item"]
        .clone();
    let b = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 B","parent_id":a["id"].as_str().unwrap()}),
        )
        .await["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 C","parent_id":root}),
        )
        .await["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let membership = f
        .ok("GET", "/envs/debug/relationship-types", Value::Null)
        .await["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["is_directory_membership"] == true)
        .map(|item| item["id"].clone())
        .unwrap();
    assert_eq!(
        f.call(
            "POST",
            "/envs/debug/relationships",
            json!({"relationship_type_id":membership,"source_entity_id":b,"target_entity_id":c}),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/envs/debug/directories/{}/move", a["id"].as_str().unwrap()),
            json!({"parent_id":b}),
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let root_move = f
        .call(
            "PATCH",
            &format!("/envs/debug/directories/{root}/move"),
            json!({"parent_id":c}),
        )
        .await;
    assert!(!root_move.0.is_success());
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/envs/debug/directories/{}", a["id"].as_str().unwrap()),
            json!({
                "name":"目录 A","is_deleted":true,
                "expected_revision":a["revision"].as_i64().unwrap()
            }),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    f.ok(
        "PATCH",
        &format!("/envs/debug/directories/{b}/move"),
        json!({"parent_id":root}),
    )
    .await;
    let tree = f
        .ok("GET", "/envs/debug/directories/tree", Value::Null)
        .await;
    let moved = tree["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == json!(b))
        .unwrap();
    assert_eq!(moved["parent_id"], json!(root));
}
