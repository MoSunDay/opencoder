use super::{absent, id, row, Server};
use serde_json::json;

#[tokio::test]
async fn cli_schema_and_environments_support_create_read_update_soft_delete() {
    let s = Server::new().await;
    let kind = s.kind().await;
    let relation = s
        .write(
            &["relationship-types", "create"],
            json!({"key":"depends","name":"依赖","source_entity_type_id":kind,
                "target_entity_type_ids":[kind]}),
        )
        .await["item"]
        .clone();
    let attribute = s
        .write(
            &["attributes", "create", &kind],
            json!({"key":"label","name":"标签","kind":"string"}),
        )
        .await["item"]
        .clone();
    let action = s
        .write(
            &["actions", "create", &kind],
            json!({"operation_type":"read","operation":"inspect","description":"查看"}),
        )
        .await["item"]
        .clone();
    let cases = [
        (
            "attributes",
            id(&attribute),
            vec!["attributes", "list", &kind],
        ),
        ("actions", id(&action), vec!["actions", "list", &kind]),
        (
            "relationship-types",
            id(&relation),
            vec!["relationship-types", "list"],
        ),
        ("entity-types", kind.as_str(), vec!["entity-types", "list"]),
    ];
    for (domain, object, list) in cases {
        let mut query = vec!["ontology"];
        query.extend(list);
        assert_eq!(row(&s.ok(&query).await, object)["revision"], 1);
        let mut update = json!({
            "name":"更新名称","description":"更新说明","required":false,
            "source_entity_type_id":kind,"target_entity_type_ids":[kind],
            "is_deleted":false,"expected_revision":1
        });
        s.write(&[domain, "update", object], update.clone()).await;
        assert_eq!(row(&s.ok(&query).await, object)["description"], "更新说明");
        s.reject(&[domain, "update", object], update.clone()).await;
        update["is_deleted"] = json!(true);
        update["expected_revision"] = json!(2);
        s.write(&[domain, "update", object], update).await;
        absent(&s.ok(&query).await, object);
        query.push("--include-deleted");
        assert_eq!(row(&s.ok(&query).await, object)["is_deleted"], true);
    }

    s.write(
        &["environments", "create"],
        json!({"key":"isolated","name":"隔离空间"}),
    )
    .await;
    for (revision, deleted) in [(1, false), (2, true)] {
        s.write(
            &["environments", "update", "isolated"],
            json!({"name":"更新空间","is_deleted":deleted,"expected_revision":revision}),
        )
        .await;
        let visible = s.ok(&["ontology", "environments", "list"]).await;
        assert_eq!(
            visible["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["env_key"] == "isolated"),
            !deleted
        );
        let all = s
            .ok(&["ontology", "environments", "list", "--include-deleted"])
            .await;
        let item = all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["env_key"] == "isolated")
            .unwrap();
        assert_eq!(item["name"], "更新空间");
        assert_eq!(item["is_deleted"], deleted);
        assert_eq!(item["revision"], revision + 1);
    }
}
