use super::{absent, id, row, Server};
use serde_json::json;

#[tokio::test]
async fn cli_entities_relationships_and_aspects_support_crud_and_graph_queries() {
    let s = Server::new().await;
    let kind = s.kind().await;
    let a = s.entity(&kind, "入口", None).await;
    let b = s.entity(&kind, "依赖", None).await;
    let relation_type = s
        .write(
            &["relationship-types", "create"],
            json!({"key":"depends","name":"依赖","source_entity_type_id":kind,
                "target_entity_type_ids":[kind]}),
        )
        .await["item"]
        .clone();
    let relation = s
        .write(
            &["relationships", "create"],
            json!({"relationship_type_id":id(&relation_type),
                "source_entity_id":id(&a),"target_entity_id":id(&b)}),
        )
        .await["item"]
        .clone();
    let all = s
        .ok(&["ontology", "entities", "list", "--limit", "500"])
        .await;
    assert_eq!(row(&all, id(&a))["name"], "入口");
    let updated = s
        .write(
            &["entities", "update", id(&a)],
            json!({"name":"新入口","is_deleted":false,"expected_revision":1}),
        )
        .await;
    assert_eq!(updated["item"]["revision"], 2);
    assert_eq!(
        s.ok(&["ontology", "entities", "get", id(&a)]).await["item"]["name"],
        "新入口"
    );
    for anchor in [id(&a), id(&b)] {
        let list = s
            .ok(&["ontology", "relationships", "list", "--anchor", anchor])
            .await;
        assert_eq!(row(&list, id(&relation))["source_entity_id"], id(&a));
        assert_eq!(list["items"].as_array().unwrap().len(), 1);
    }
    s.write(
        &["relationships", "update", id(&relation)],
        json!({"description":"显式依赖","is_pinned":true,
            "is_deleted":false,"expected_revision":1}),
    )
    .await;
    let list = s.ok(&["ontology", "relationships", "list"]).await;
    assert_eq!(row(&list, id(&relation))["description"], "显式依赖");
    assert_eq!(row(&list, id(&relation))["is_pinned"], true);
    let graph_args = [
        "ontology",
        "graph",
        "--center",
        id(&a),
        "--entity-type-id",
        &kind,
        "--relationship-type-id",
        id(&relation_type),
        "--upstream-depth",
        "0",
        "--downstream-depth",
        "1",
    ];
    let graph = s.ok(&graph_args).await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"][0]["id"], id(&relation));

    let mut aspect_body = json!({
        "aspect_key":"dependencies","name":"依赖切面","entity_type_ids":[kind],
        "relationship_type_ids":[id(&relation_type)],"default_center_ids":[id(&a)],
        "default_upstream_depth":0,"default_downstream_depth":1
    });
    let aspect = s.write(&["aspects", "create"], aspect_body.clone()).await["item"].clone();
    assert_eq!(aspect["aspect_key"], "dependencies");
    let list = s.ok(&["ontology", "aspects", "list"]).await;
    assert_eq!(row(&list, id(&aspect))["entity_type_ids"], json!([kind]));
    aspect_body["name"] = json!("更新切面");
    aspect_body["default_downstream_depth"] = json!(2);
    aspect_body["is_deleted"] = json!(false);
    aspect_body["expected_revision"] = json!(1);
    s.write(&["aspects", "update", id(&aspect)], aspect_body)
        .await;
    let detail = s.ok(&["ontology", "aspects", "get", id(&aspect)]).await;
    assert_eq!(detail["item"]["name"], "更新切面");
    assert_eq!(detail["item"]["default_downstream_depth"], 2);
    assert_eq!(
        s.call(&[
            "ontology",
            "aspects",
            "delete",
            id(&aspect),
            "--expected-revision",
            "1"
        ])
        .await
        .0,
        4
    );
    s.ok(&[
        "ontology",
        "aspects",
        "delete",
        id(&aspect),
        "--expected-revision",
        "2",
    ])
    .await;
    absent(&s.ok(&["ontology", "aspects", "list"]).await, id(&aspect));
    let list = s
        .ok(&["ontology", "aspects", "list", "--include-deleted"])
        .await;
    assert_eq!(row(&list, id(&aspect))["is_deleted"], true);

    s.write(
        &["relationships", "update", id(&relation)],
        json!({"description":"显式依赖","is_deleted":true,"expected_revision":2}),
    )
    .await;
    absent(
        &s.ok(&["ontology", "relationships", "list"]).await,
        id(&relation),
    );
    let list = s
        .ok(&[
            "ontology",
            "relationships",
            "list",
            "--anchor",
            id(&a),
            "--include-deleted",
        ])
        .await;
    assert_eq!(row(&list, id(&relation))["is_deleted"], true);
    assert_eq!(s.ok(&graph_args).await["edges"], json!([]));
    s.write(
        &["entities", "update", id(&a)],
        json!({"name":"新入口","is_deleted":true,"expected_revision":2}),
    )
    .await;
    absent(
        &s.ok(&["ontology", "entities", "list", "--limit", "500"])
            .await,
        id(&a),
    );
    let list = s
        .ok(&[
            "ontology",
            "entities",
            "list",
            "--limit",
            "500",
            "--include-deleted",
        ])
        .await;
    assert_eq!(row(&list, id(&a))["is_deleted"], true);
}
