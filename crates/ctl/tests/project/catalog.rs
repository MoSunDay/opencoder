use super::server_local_defs::{api_get, assert_ok, cli, Server, TOKEN};
use serde_json::json;

#[tokio::test]
async fn project_catalog_tags_and_board_order_roundtrip() {
    let s = Box::pin(Server::new(None)).await;
    assert_ok(
        &s,
        &[
            "project",
            "goals",
            "create",
            "--json",
            r#"{"title":"project"}"#,
        ],
    )
    .await;
    let goals = api_get(&s, "/api/project/goals").await;
    let goal = goals["goals"][0]["id"].as_str().unwrap();
    assert_ok(&s, &["project", "goals", "get", goal]).await;
    let body = json!({"goal_id":goal,"title":"initiative"}).to_string();
    assert_ok(&s, &["project", "initiatives", "create", "--json", &body]).await;
    let groups = api_get(&s, "/api/project/initiatives").await;
    let group = groups["initiatives"][0]["id"].as_str().unwrap();
    assert_ok(&s, &["project", "initiatives", "get", group]).await;
    let tag_body = json!({"scope_type":"project","scope_id":goal,"name":"priority"}).to_string();
    assert_ok(&s, &["project", "tags", "create", "--json", &tag_body]).await;
    assert_ok(
        &s,
        &[
            "project",
            "tags",
            "list",
            "--scope-type",
            "project",
            "--scope-id",
            goal,
        ],
    )
    .await;
    let tags = api_get(&s, "/api/project/tags").await;
    let tag = tags["tags"][0]["id"].as_str().unwrap();
    let tag_patch = json!({"scope_type":"project","scope_id":goal,"name":"urgent"}).to_string();
    assert_ok(&s, &["project", "tags", "patch", tag, "--json", &tag_patch]).await;
    for title in ["first", "second"] {
        let body =
            json!({"title":title,"draft":"task details","initiative_id":group,"tag_ids":[tag]})
                .to_string();
        assert_ok(&s, &["project", "todos", "create", "--json", &body]).await;
    }
    let todos = api_get(&s, "/api/project/todos").await;
    let ids: Vec<_> = todos["todos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_ok(&s, &["project", "todos", "get", ids[0]]).await;
    let body =
        json!({"initiative_id":group,"board_status":"backlog","ids":[ids[1],ids[0]]}).to_string();
    assert_ok(&s, &["project", "todos", "reorder", "--json", &body]).await;
    let first = api_get(&s, &format!("/api/project/todos/{}", ids[1])).await;
    assert_eq!(first["tag_ids"], json!([tag]));
    assert_eq!(first["position"], 1000);
    assert_eq!(
        cli(&s, TOKEN, &["project", "todos", "messages", ids[0]]).await,
        4,
        "reading must not launch an execution"
    );
    assert_ok(&s, &["project", "tags", "delete", tag]).await;
    for id in ids {
        assert_ok(&s, &["project", "todos", "delete", id]).await;
    }
    assert_ok(&s, &["project", "initiatives", "delete", group]).await;
    assert_ok(&s, &["project", "goals", "delete", goal]).await;
}
