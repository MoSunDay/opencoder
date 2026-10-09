mod common;
use axum::http::StatusCode;
use common::Fixture;
use opencoder_core::identity::Role;
use serde_json::{json, Value};

#[tokio::test]
async fn graph_aspects_relationship_scopes_and_directory_cycles_use_live_metadata() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let a = f.entity(&kind, "入口").await;
    let b = f.entity(&kind, "下游").await;
    let a = a["id"].as_str().unwrap();
    let b = b["id"].as_str().unwrap();
    let rel=f.ok("POST","/envs/debug/relationship-types",json!({"key":"calls","name":"调用","source_entity_type_id":kind,"target_entity_type_ids":[kind]})).await["item"].clone();
    let rel = rel["id"].as_str().unwrap();
    let edge = f
        .ok(
            "POST",
            "/envs/debug/relationships",
            json!({"relationship_type_id":rel,"source_entity_id":a,"target_entity_id":b}),
        )
        .await["item"]
        .clone();
    let graph=f.ok("GET",&format!("/envs/debug/graph?center={a}&entity_type_id={kind}&relationship_type_id={rel}&upstream_depth=0&downstream_depth=1"),Value::Null).await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    let aspect=f.ok("POST","/envs/debug/graph-aspects",json!({"key":"service_calls","name":"服务调用","entity_type_ids":[kind],"relationship_type_ids":[rel],"default_center_ids":[a],"default_upstream_depth":0,"default_downstream_depth":1})).await["item"].clone();
    assert_eq!(aspect["entity_type_ids"], json!([kind]));
    let path = format!(
        "/envs/debug/graph-aspects/{}",
        aspect["id"].as_str().unwrap()
    );
    assert_eq!(
        f.call(
            "DELETE",
            &format!("{path}?expected_revision=0"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    f.ok(
        "DELETE",
        &format!("{path}?expected_revision=1"),
        Value::Null,
    )
    .await;
    assert_eq!(
        f.ok("GET", "/envs/debug/graph-aspects", Value::Null).await["items"],
        json!([])
    );
    let root = f
        .ok("GET", "/envs/debug/directories/tree", Value::Null)
        .await["root_id"]
        .as_str()
        .unwrap()
        .to_string();
    let parent = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 A","parent_id":root}),
        )
        .await;
    let parent = parent["item"]["id"].as_str().unwrap();
    let child = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"目录 B","parent_id":parent}),
        )
        .await;
    let child = child["item"]["id"].as_str().unwrap();
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/envs/debug/directories/{parent}/move"),
            json!({"parent_id":child})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let root_move = f
        .call(
            "PATCH",
            &format!("/envs/debug/directories/{root}/move"),
            json!({"parent_id":parent}),
        )
        .await;
    assert!(!root_move.0.is_success());
    f.ok(
        "PATCH",
        &format!("/envs/debug/relationships/{}", edge["id"].as_str().unwrap()),
        json!({"description":"禁用","is_deleted":true,"is_pinned":false,"expected_revision":1}),
    )
    .await;
    let graph = f
        .ok(
            "GET",
            &format!("/envs/debug/graph?center={a}&entity_type_id={kind}&expand_neighbors=true"),
            Value::Null,
        )
        .await;
    assert_eq!(graph["edges"], json!([]));
}

#[tokio::test]
async fn viewer_reads_all_domains_but_cannot_modify_them() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let entity = f.entity(&kind, "只读").await;
    let id = entity["id"].as_str().unwrap();
    let role = Role::Viewer;
    for (role, capabilities) in [
        (Role::Viewer, json!(["view_ontology"])),
        (Role::Editor, json!(["view_ontology", "manage_ontology"])),
        (Role::Admin, json!(["view_ontology", "manage_ontology"])),
    ] {
        let (status, session) = common::call(&f.router, role, "GET", "/session", Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(session["capabilities"], capabilities);
    }
    for path in [
        "/session".to_string(),
        "/environments".into(),
        "/envs/debug/entity-types".into(),
        "/envs/debug/entities".into(),
        "/envs/debug/relationship-types".into(),
        "/envs/debug/relationships".into(),
        "/envs/debug/directories/tree".into(),
        "/envs/debug/graph-aspects".into(),
        format!("/envs/debug/entities/{id}"),
    ] {
        assert_eq!(
            common::call(&f.router, role, "GET", &path, Value::Null)
                .await
                .0,
            StatusCode::OK,
            "{path}"
        );
    }
    assert_eq!(
        common::call(
            &f.router,
            role,
            "POST",
            "/environments",
            json!({"key":"no","name":"禁止"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        common::call(
            &f.router,
            role,
            "PATCH",
            &format!("/envs/debug/entities/{id}"),
            json!({"name":"禁止","is_deleted":false,"expected_revision":1})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn vectors_search_real_cosine_values_and_keep_environment_partitions() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let e = f.entity(&kind, "向量").await;
    let attr = f
        .ok(
            "POST",
            &format!("/envs/debug/entity-types/{kind}/attributes"),
            json!({"key":"embedding","name":"向量","kind":"vector","required":false}),
        )
        .await["item"]
        .clone();
    let attr = attr["id"].as_str().unwrap();
    let mut vector = vec![0f32; 2048];
    vector[0] = 1.;
    let path = format!(
        "/envs/debug/entities/{}/vectors/{attr}",
        e["id"].as_str().unwrap()
    );
    let saved = f
        .ok("PUT", &path, json!({"vector":vector,"expected_revision":0}))
        .await;
    let result = f
        .ok(
            "POST",
            "/envs/debug/vector-search",
            json!({"attribute_definition_id":attr.parse::<i64>().unwrap(),"vector":vector}),
        )
        .await;
    assert_eq!(result["items"][0]["vector_id"], saved["vector_id"]);
    assert!((result["items"][0]["similarity"].as_f64().unwrap() - 1.).abs() < 1e-6);
    assert_eq!(
        f.call("PUT", &path, json!({"vector":vector,"expected_revision":0}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.ok(
        "POST",
        "/environments",
        json!({"key":"other","name":"其他"}),
    )
    .await;
    assert_eq!(
        f.ok(
            "POST",
            "/envs/other/vector-search",
            json!({"attribute_definition_id":attr.parse::<i64>().unwrap(),"vector":vector})
        )
        .await["items"],
        json!([])
    );
}

#[tokio::test]
async fn editor_manages_ontology_environments_and_entities() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let entity = f.entity(&kind, "before").await;
    let (status, _) = common::call(
        &f.router,
        Role::Editor,
        "POST",
        "/environments",
        json!({"key":"editor_env","name":"Editor environment"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = common::call(
        &f.router,
        Role::Editor,
        "PATCH",
        &format!("/envs/debug/entities/{}", entity["id"].as_str().unwrap()),
        json!({"name":"after","is_deleted":false,"expected_revision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["item"]["name"], "after");
}
