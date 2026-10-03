mod common;
use axum::http::StatusCode;
use common::{creation, Fixture};
use serde_json::{json, Value};

#[tokio::test]
async fn creation_is_atomic_idempotent_and_preserves_text_history_after_restart() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let body = creation(&kind, "订单服务");
    // Force a storage failure after the request claim and staged entity exist.
    std::fs::write(f.directory.path().join("files/debug"), b"blocked").unwrap();
    assert!(!f
        .call("POST", "/envs/debug/entities", body.clone())
        .await
        .0
        .is_success());
    let conn = f.connection().await;
    let count = conn
        .query(
            "SELECT count(*) FROM entities WHERE json_extract(body,'$.entity_type_id')=?1",
            [kind.clone()],
        )
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get::<i64>(0)
        .unwrap();
    assert_eq!(count, 0);
    let claims = conn.query("SELECT count(*) FROM audit_events WHERE json_extract(body,'$.action')='entity.create.request'",()).await.unwrap().next().await.unwrap().unwrap().get::<i64>(0).unwrap();
    assert_eq!(claims, 0);
    std::fs::remove_file(f.directory.path().join("files/debug")).unwrap();
    let (first, second) = tokio::join!(
        f.ok("POST", "/envs/debug/entities", body.clone()),
        f.ok("POST", "/envs/debug/entities", body.clone())
    );
    assert_eq!(first, second);
    let mut changed = body;
    changed["name"] = json!("冲突");
    assert_eq!(
        f.call("POST", "/envs/debug/entities", changed).await.0,
        StatusCode::CONFLICT
    );
    let id = first["item"]["id"].as_str().unwrap();
    let detail = f
        .ok("GET", &format!("/envs/debug/entities/{id}"), Value::Null)
        .await;
    assert_eq!(detail["needs_completion"], false);
    let source = detail["text_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["definition"]["attribute_role"] == "source")
        .unwrap();
    let attr = source["definition"]["id"].as_str().unwrap();
    let path = format!("/envs/debug/entities/{id}/attributes/{attr}/text");
    let updated = f
        .ok(
            "PUT",
            &path,
            json!({"content":"# 新来源","format":"md","expected_revision":1}),
        )
        .await;
    assert_eq!(updated["revision"], 2);
    assert_eq!(
        f.call(
            "PUT",
            &path,
            json!({"content":"old","format":"md","expected_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    f.state.drained().await;
    let reopened = opencoder_ontology::AppState::open(
        &f.directory.path().join("ontology.db"),
        &f.directory.path().join("files"),
    )
    .await
    .unwrap();
    reopened.ready().await.unwrap();
    let router = opencoder_ontology::router(reopened);
    let (status, old) = common::call(
        &router,
        opencoder_core::identity::Role::Admin,
        "GET",
        &format!("{path}/1"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(old["content"], "# 来源");
    let history = f.ok("GET", &path, Value::Null).await;
    assert_eq!(history["items"].as_array().unwrap().len(), 2);
    assert_eq!(history["items"][0]["is_current"], true);
    assert_eq!(history["items"][1]["is_current"], false);
    assert_ne!(
        history["items"][0]["content_path"],
        history["items"][1]["content_path"]
    );
}

#[tokio::test]
async fn environments_and_required_attributes_are_isolated_and_revision_checked() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    f.ok("POST", "/environments", json!({"key":"prod","name":"生产"}))
        .await;
    assert_eq!(
        f.ok("GET", "/envs/prod/entities", Value::Null).await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let attr = f
        .ok(
            "POST",
            &format!("/envs/debug/entity-types/{kind}/attributes"),
            json!({"key":"enabled","name":"启用","kind":"boolean","required":true}),
        )
        .await["item"]
        .clone();
    let mut body = creation(&kind, "服务");
    assert_eq!(
        f.call("POST", "/envs/debug/entities", body.clone()).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    body["attributes"][attr["id"].as_str().unwrap()] = json!(false);
    let entity = f.ok("POST", "/envs/debug/entities", body).await["item"].clone();
    let path = format!(
        "/envs/debug/entities/{}/attributes/{}",
        entity["id"].as_str().unwrap(),
        attr["id"].as_str().unwrap()
    );
    assert_eq!(
        f.call(
            "PUT",
            &path,
            json!({"kind":"boolean","value":false,"is_deleted":true,"expected_revision":1})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        f.call(
            "GET",
            &format!("/envs/prod/entities/{}", entity["id"].as_str().unwrap()),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let action = f
        .ok(
            "POST",
            &format!("/envs/debug/entity-types/{kind}/actions"),
            json!({"operation_type":"read","operation":"inspect","description":"查看"}),
        )
        .await["item"]
        .clone();
    assert!(action["id"].is_string());
    let updated = f
        .ok(
            "PATCH",
            &format!("/envs/debug/actions/{}", action["id"].as_str().unwrap()),
            json!({"description":"新版","is_deleted":false,"expected_revision":1}),
        )
        .await;
    assert_eq!(updated["item"]["revision"], 2);
    assert_eq!(
        f.call(
            "PATCH",
            "/environments/debug",
            json!({"name":"debug","is_deleted":true,"expected_revision":1})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
