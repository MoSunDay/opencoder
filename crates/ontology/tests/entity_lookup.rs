mod common;
use common::{creation, Fixture};
use serde_json::{json, Value};

#[tokio::test]
async fn entity_reads_preserve_scope_and_revisions_with_unrelated_values() {
    let f = Fixture::new().await;
    let kind = f.kind().await;
    let attr = f
        .ok(
            "POST",
            &format!("/envs/debug/entity-types/{kind}/attributes"),
            json!({"key":"payload","name":"Payload","kind":"json","required":false}),
        )
        .await;
    let attr_id = attr["item"]["id"].as_str().unwrap();
    let directory = f
        .ok(
            "POST",
            "/envs/debug/directories/tree",
            json!({"name":"Objects","parent_id":"00000000-0000-4000-8000-000000000002"}),
        )
        .await;
    let start = std::time::Instant::now();
    let mut entities = Vec::new();
    for i in 0..300 {
        let mut body = creation(&kind, &format!("Object {i}"));
        body["directory_id"] = directory["item"]["id"].clone();
        body["attributes"][attr_id] = json!({"sequence":i,"content":"value".repeat(100)});
        entities.push(f.ok("POST", "/envs/debug/entities", body).await["item"].clone());
    }
    eprintln!(
        "created 300 entities with structured values and text in {:?}",
        start.elapsed()
    );
    for index in [0, 149, 299] {
        let id = entities[index]["id"].as_str().unwrap();
        let path = format!("/envs/debug/entities/{id}");
        let detail = f.ok("GET", &path, Value::Null).await;
        assert_eq!(detail["structured_attributes"].as_array().unwrap().len(), 1);
        assert_eq!(
            detail["structured_attributes"][0]["value"]["sequence"],
            index
        );
        assert_eq!(detail["needs_completion"], false);
        let source = detail["text_attributes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["definition"]["attribute_role"] == "source")
            .unwrap();
        let source_id = source["definition"]["id"].as_str().unwrap();
        let text_path = format!("{path}/attributes/{source_id}/text");
        f.ok(
            "PUT",
            &text_path,
            json!({"format":"md","content":"updated","expected_revision":1}),
        )
        .await;
        let old = f.ok("GET", &format!("{text_path}/1"), Value::Null).await;
        assert_eq!(old["content"], "# 来源");
        let history = f.ok("GET", &text_path, Value::Null).await;
        assert_eq!(history["items"].as_array().unwrap().len(), 2);
        let relations = f
            .ok(
                "GET",
                &format!("/envs/debug/relationships?anchor={id}"),
                Value::Null,
            )
            .await;
        assert_eq!(relations["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            relations["items"][0]["target_entity_id"],
            directory["item"]["id"]
        );
    }
}
