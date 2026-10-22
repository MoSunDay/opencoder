#[path = "ontology_server/mod.rs"]
mod support;

use serde_json::json;
use support::Server;

#[tokio::test]
async fn cli_catalog_and_data_commands_roundtrip_through_real_server() {
    let s = Server::new().await;
    s.ok(&["ontology", "sync", "run"]).await;
    assert_eq!(s.ok(&["ontology", "sync", "check"]).await["in_sync"], true);
    let list = s
        .ok(&["ontology", "capabilities", "list", "--kind", "operator"])
        .await;
    assert_eq!(list["total"], 1);
    let id = list["items"][0]["id"].as_str().unwrap();
    let detail = s.ok(&["ontology", "entities", "get", id]).await;
    assert_eq!(detail["management"]["managed_by"], "platform-catalog");
    let (code, _) = s
        .call(&[
            "ontology",
            "entities",
            "update",
            id,
            "--json",
            r#"{"name":"changed","is_deleted":false,"expected_revision":1}"#,
        ])
        .await;
    assert_eq!(code, 4);
    let kind = s
        .ok(&[
            "ontology",
            "entity-types",
            "create",
            "--json",
            r#"{"key":"custom","name":"Custom"}"#,
        ])
        .await;
    let kind = kind["item"]["id"].as_str().unwrap();
    let file = s.file(
        "entity.json",
        json!({"request_id":"00000000-0000-4000-8000-000000000099",
        "entity_type_id":kind,"name":"Example","source":"Source","ext":"Notes","attributes":{}}),
    );
    let entity = s
        .ok(&[
            "ontology",
            "entities",
            "create",
            "--json",
            &format!("@{}", file.display()),
        ])
        .await;
    let entity = entity["item"]["id"].as_str().unwrap();
    let detail = s.ok(&["ontology", "entities", "get", entity]).await;
    let source = detail["text_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["definition"]["attribute_role"] == "source")
        .unwrap();
    let attribute = source["definition"]["id"].as_str().unwrap();
    s.ok(&[
        "ontology",
        "entities",
        "set-text",
        entity,
        attribute,
        "--json",
        r#"{"format":"md","content":"Updated","expected_revision":1}"#,
    ])
    .await;
    assert_eq!(
        s.ok(&["ontology", "entities", "text-get", entity, attribute, "2"])
            .await["content"],
        "Updated"
    );
    assert_eq!(
        s.call(&[
            "ontology",
            "entities",
            "set-text",
            entity,
            attribute,
            "--json",
            r#"{"format":"md","content":"Stale","expected_revision":1}"#
        ])
        .await
        .0,
        4
    );
    let changed = json!({"name":"new-team","captain":"act","members":[{"agent":"act"}]});
    s.state
        .fleet
        .put_definition("team", "new-team", &changed)
        .await
        .unwrap();
    assert_eq!(s.call(&["ontology", "sync", "check"]).await.0, 4);
    s.ok(&["ontology", "sync", "run"]).await;
    assert_eq!(s.ok(&["ontology", "sync", "check"]).await["in_sync"], true);
}
