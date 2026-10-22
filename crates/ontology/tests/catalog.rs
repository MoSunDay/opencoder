mod common;
use common::Fixture;
use opencoder_core::ontology::Capability;
use opencoder_ontology::CatalogOperation as Op;
use serde_json::{json, Value};

fn entry(key: &str) -> Capability {
    Capability {
        key: key.into(),
        source: "test".into(),
        source_id: key.into(),
        kind: "agent".into(),
        name: format!("Capability {key}"),
        description: "Public capability description".into(),
        cli: vec!["opencoder-cli agents --help".into()],
        ..Default::default()
    }
}
async fn apply(f: &Fixture, entries: Vec<Capability>) -> Value {
    f.state
        .catalog("debug".into(), Op::Apply(entries))
        .await
        .unwrap()
}
async fn rows(f: &Fixture) -> Value {
    f.state.catalog("debug".into(), Op::List).await.unwrap()
}

#[tokio::test]
async fn catalog_is_idempotent_and_preserves_notes_and_manual_relations() {
    let f = Fixture::new().await;
    let mut a = entry("a");
    a.dependencies.push("b".into());
    let b = entry("b");
    assert_eq!(apply(&f, vec![a.clone(), b.clone()]).await["created"], 2);
    let items = rows(&f).await;
    let id = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["capability"]["key"] == "a")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let path = format!("/envs/debug/entities/{id}");
    let detail = f.ok("GET", &path, Value::Null).await;
    assert_eq!(detail["management"]["managed_by"], "platform-catalog");
    let graph = f
        .ok(
            "GET",
            &format!("/envs/debug/graph?center={id}&entity_type_id={}&relationship_type_id=00000000-0000-4000-8000-000000000012&downstream_depth=1", detail["item"]["entity_type_id"].as_str().unwrap()),
            Value::Null,
        )
        .await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.ok("GET", "/envs/debug/graph-aspects", Value::Null).await["items"][0]["aspect_key"],
        "platform_capabilities"
    );
    let notes = detail["text_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["definition"]["attribute_role"] == "ext")
        .unwrap();
    let attr = notes["definition"]["id"].as_str().unwrap();
    f.ok("PUT", &format!("{path}/attributes/{attr}/text"),
        json!({"format":"md","content":"User notes","expected_revision":notes["current"]["revision"]})).await;
    let kind = f.kind().await;
    let other = f.entity(&kind, "Manual entity").await;
    let relation = f.ok("POST","/envs/debug/relationships",json!({
        "relationship_type_id":"00000000-0000-4000-8000-000000000012",
        "source_entity_id":id,"target_entity_id":other["id"],"description":"Manual dependency"
    })).await;
    let no_op = apply(&f, vec![a.clone(), b.clone()]).await;
    assert_eq!(no_op["updated"], 0);
    assert_eq!(
        f.ok("GET", &path, Value::Null).await["item"]["revision"],
        detail["item"]["revision"]
    );
    a.description = "Updated public description".into();
    assert_eq!(apply(&f, vec![a, b]).await["updated"], 1);
    let text = f
        .ok(
            "GET",
            &format!("{path}/attributes/{attr}/text/2"),
            Value::Null,
        )
        .await;
    assert_eq!(text["content"], "User notes");
    let relations = f.ok("GET", "/envs/debug/relationships", Value::Null).await;
    assert!(relations["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == relation["item"]["id"]));
    assert!(
        f.state
            .catalog("debug".into(), Op::Check(vec![entry("different")]))
            .await
            .unwrap()["in_sync"]
            == false
    );
}

#[tokio::test]
async fn managed_fields_reject_writes_but_custom_attributes_remain_editable() {
    let f = Fixture::new().await;
    apply(&f, vec![entry("a")]).await;
    let items = rows(&f).await;
    let id = items["items"][0]["id"].as_str().unwrap();
    let path = format!("/envs/debug/entities/{id}");
    let detail = f.ok("GET", &path, Value::Null).await;
    let (status, error) = f
        .call(
            "PATCH",
            &path,
            json!({"name":"overwrite","is_deleted":false,"expected_revision":1}),
        )
        .await;
    assert_eq!(status.as_u16(), 409);
    assert!(error["error"].as_str().unwrap().contains("原模块"));
    for attribute in ["91", "92", "091", "%2B92"] {
        assert_eq!(
            f.call(
                "PUT",
                &format!("{path}/attributes/{attribute}"),
                json!({"kind":"json","value":{},"expected_revision":1})
            )
            .await
            .0
            .as_u16(),
            409
        );
    }
    let encoded = path.replacen("/entities/", "/entities/%", 1);
    // A percent-encoded first UUID byte still resolves to the protected entity.
    let encoded = encoded.replace(
        &format!("%{}", &id[..1]),
        &format!("%{:02X}", id.as_bytes()[0]),
    );
    assert_eq!(
        f.call(
            "PATCH",
            &encoded,
            json!({"name":"overwrite","is_deleted":false,"expected_revision":1})
        )
        .await
        .0
        .as_u16(),
        409
    );
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/envs/debug/entities/{}", id.to_uppercase()),
            json!({"name":"overwrite","is_deleted":false,"expected_revision":1})
        )
        .await
        .0
        .as_u16(),
        409
    );
    let kind = detail["item"]["entity_type_id"].as_str().unwrap();
    let attribute = f
        .ok(
            "POST",
            &format!("/envs/debug/entity-types/{kind}/attributes"),
            json!({"key":"annotation","name":"Annotation","kind":"string"}),
        )
        .await;
    f.ok(
        "PUT",
        &format!(
            "{path}/attributes/{}",
            attribute["item"]["id"].as_str().unwrap()
        ),
        json!({"kind":"string","value":"custom","expected_revision":0}),
    )
    .await;
}

#[tokio::test]
async fn deletion_and_restore_keep_entity_identity_and_missing_dependencies_are_visible() {
    let f = Fixture::new().await;
    let mut a = entry("a");
    a.dependencies.push("missing".into());
    assert_eq!(
        apply(&f, vec![a.clone()]).await["issues"][0]["missing_dependency"],
        "missing"
    );
    let id = rows(&f).await["items"][0]["id"].clone();
    assert_eq!(apply(&f, vec![]).await["deleted"], 1);
    assert_eq!(rows(&f).await["items"][0]["is_deleted"], true);
    assert_eq!(apply(&f, vec![a]).await["updated"], 1);
    assert_eq!(rows(&f).await["items"][0]["id"], id);
    assert_eq!(rows(&f).await["items"][0]["is_deleted"], false);
}

#[tokio::test]
async fn failed_snapshot_rolls_back_changes_and_retains_last_success_on_error() {
    let f = Fixture::new().await;
    let original = entry("a");
    let status = apply(&f, vec![original.clone()]).await;
    let mut changed = original.clone();
    changed.description = "Changed".into();
    let invalid = entry("b");
    use sha2::{Digest, Sha256};
    let id = uuid::Uuid::from_bytes(
        Sha256::digest(b"platform-catalog/entity/b")[..16]
            .try_into()
            .unwrap(),
    )
    .to_string();
    let connection = f.connection().await;
    connection
        .execute(
            "INSERT INTO entities(env_num,id,body) VALUES(1,?1,?2)",
            turso::params![
                id.clone(),
                json!({"id":id,"name":"Unmanaged collision","revision":1}).to_string()
            ],
        )
        .await
        .unwrap();
    assert!(f
        .state
        .catalog("debug".into(), Op::Apply(vec![changed.clone(), invalid]))
        .await
        .is_err());
    assert_eq!(rows(&f).await["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        rows(&f).await["items"][0]["capability"]["description"],
        original.description
    );
    let error = f
        .state
        .catalog("debug".into(), Op::Error("source unavailable".into()))
        .await
        .unwrap();
    assert_eq!(error["last_success"], status["last_success"]);
    assert_eq!(error["state"], "error");
    connection
        .execute("DELETE FROM entities WHERE env_num=1 AND id=?1", [id])
        .await
        .unwrap();
    changed.description = "Different update after rollback".into();
    assert_eq!(apply(&f, vec![changed]).await["state"], "ready");
}

#[tokio::test]
async fn catalog_reopens_without_rebinding_storage_and_is_environment_isolated() {
    let f = Fixture::new().await;
    apply(&f, vec![entry("a")]).await;
    f.ok(
        "POST",
        "/environments",
        json!({"key":"other","name":"Other"}),
    )
    .await;
    assert_eq!(
        f.state.catalog("other".into(), Op::List).await.unwrap()["items"],
        json!([])
    );
    let reopened = opencoder_ontology::AppState::open(
        &f.directory.path().join("ontology.db"),
        &f.directory.path().join("files"),
    )
    .await
    .unwrap();
    assert_eq!(
        reopened
            .catalog("debug".into(), Op::Check(vec![entry("a")]))
            .await
            .unwrap()["in_sync"],
        true
    );
    assert!(reopened.catalog("unknown".into(), Op::List).await.is_err());
    assert!(reopened
        .catalog("debug".into(), Op::Apply(vec![entry("a"), entry("a")]))
        .await
        .is_err());
}

#[tokio::test]
async fn check_detects_changed_generated_objects_and_sync_repairs_them() {
    let f = Fixture::new().await;
    apply(&f, vec![entry("a")]).await;
    let id = rows(&f).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/envs/debug/entities/{id}");
    let mut value = f.ok("GET", &path, Value::Null).await["item"].clone();
    value["description"] = json!("Unexpected stored value");
    let conn = f.connection().await;
    conn.execute(
        "UPDATE entities SET body=?1 WHERE env_num=1 AND id=?2",
        turso::params![value.to_string(), id.clone()],
    )
    .await
    .unwrap();
    let check = f
        .state
        .catalog("debug".into(), Op::Check(vec![entry("a")]))
        .await
        .unwrap();
    assert_eq!(check["in_sync"], false);
    assert!(check["drifted"]
        .as_array()
        .unwrap()
        .contains(&json!(format!("entities:{id}"))));
    assert_eq!(apply(&f, vec![entry("a")]).await["repaired"], 1);
    assert_eq!(
        f.ok("GET", &path, Value::Null).await["item"]["description"],
        entry("a").description
    );
    let ownership = format!("entities:{id}");
    conn.execute(
        "UPDATE catalog_objects SET body=?1 WHERE env_num=1 AND id=?2",
        turso::params![
            json!({"table":"entities","id":id,"fingerprint":"outdated"}).to_string(),
            ownership
        ],
    )
    .await
    .unwrap();
    assert_eq!(apply(&f, vec![entry("a")]).await["repaired"], 1);
    assert_eq!(
        f.state
            .catalog("debug".into(), Op::Check(vec![entry("a")]))
            .await
            .unwrap()["in_sync"],
        true
    );
}
