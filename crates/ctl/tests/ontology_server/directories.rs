use super::{absent, id, row, Server};
use serde_json::json;

#[tokio::test]
async fn cli_directory_tree_mounts_new_and_existing_entities_and_rejects_invalid_moves() {
    let s = Server::new().await;
    let kind = s.kind().await;
    let tree = s.ok(&["ontology", "directories", "tree"]).await;
    let root = tree["root_id"].as_str().unwrap();
    let parent = s
        .write(
            &["directories", "create"],
            json!({"name":"目录 A","parent_id":root}),
        )
        .await["item"]
        .clone();
    let child = s
        .write(
            &["directories", "create"],
            json!({"name":"目录 B","parent_id":id(&parent)}),
        )
        .await["item"]
        .clone();
    let tree = s.ok(&["ontology", "directories", "tree"]).await;
    assert_eq!(row(&tree, id(&child))["parent_id"], id(&parent));
    s.reject(
        &["directories", "move", id(&parent)],
        json!({"parent_id":id(&child)}),
    )
    .await;
    s.reject(
        &["directories", "move", root],
        json!({"parent_id":id(&parent)}),
    )
    .await;

    // Discover the built-in relationship instead of depending on a display name.
    let types = s.ok(&["ontology", "relationship-types", "list"]).await;
    let membership = types["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["is_directory_membership"] == true)
        .unwrap();
    let created = s.entity(&kind, "创建时挂载", Some(id(&child))).await;
    let existing = s.entity(&kind, "已有实体", None).await;
    let mount = s
        .write(
            &["relationships", "create"],
            json!({
                "relationship_type_id":id(membership),
                "source_entity_id":id(&existing),"target_entity_id":id(&child)
            }),
        )
        .await["item"]
        .clone();
    let contents = s
        .ok(&["ontology", "relationships", "list", "--anchor", id(&child)])
        .await;
    for entity in [&created, &existing] {
        assert!(contents["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|relation| {
                relation["relationship_type_id"] == id(membership)
                    && relation["source_entity_id"] == id(entity)
                    && relation["target_entity_id"] == id(&child)
            }));
    }
    s.reject(
        &["relationships", "create"],
        json!({
            "relationship_type_id":id(membership),
            "source_entity_id":id(&existing),"target_entity_id":id(&parent)
        }),
    )
    .await;
    s.reject(
        &["directories", "update", id(&child)],
        json!({
            "name":"目录 B","is_deleted":true,"expected_revision":1
        }),
    )
    .await;
    s.write(
        &["directories", "move", id(&child)],
        json!({"parent_id":root}),
    )
    .await;
    s.write(
        &["directories", "update", id(&child)],
        json!({
            "name":"新目录 B","is_deleted":false,"expected_revision":1
        }),
    )
    .await;
    let tree = s.ok(&["ontology", "directories", "tree"]).await;
    assert_eq!(row(&tree, id(&child))["name"], "新目录 B");
    assert_eq!(row(&tree, id(&child))["parent_id"], root);

    s.write(
        &["relationships", "update", id(&mount)],
        json!({
            "is_deleted":true,"expected_revision":1
        }),
    )
    .await;
    let remounted = s
        .write(
            &["relationships", "create"],
            json!({
                "relationship_type_id":id(membership),
                "source_entity_id":id(&existing),"target_entity_id":id(&parent)
            }),
        )
        .await["item"]
        .clone();
    let contents = s
        .ok(&["ontology", "relationships", "list", "--anchor", id(&child)])
        .await;
    absent(&contents, id(&mount));
    let contents = s
        .ok(&[
            "ontology",
            "relationships",
            "list",
            "--anchor",
            id(&existing),
        ])
        .await;
    assert_eq!(
        row(&contents, id(&remounted))["target_entity_id"],
        id(&parent)
    );
    s.write(
        &["relationships", "update", id(&remounted)],
        json!({
            "is_deleted":true,"expected_revision":1
        }),
    )
    .await;
    s.write(
        &["directories", "update", id(&parent)],
        json!({
            "name":"目录 A","is_deleted":true,"expected_revision":1
        }),
    )
    .await;
    absent(
        &s.ok(&["ontology", "directories", "tree"]).await,
        id(&parent),
    );
    let tree = s
        .ok(&["ontology", "directories", "tree", "--include-deleted"])
        .await;
    assert_eq!(row(&tree, id(&parent))["is_deleted"], true);
}
