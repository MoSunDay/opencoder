mod common;

use axum::http::StatusCode;
use common::Fixture;
use opencoder_ontology::AppState;
use serde_json::{json, Value};

async fn binding(fixture: &Fixture) -> String {
    fixture
        .connection()
        .await
        .query(
            "SELECT files_root FROM ontology_schema_version WHERE version=1",
            (),
        )
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get(0)
        .unwrap()
}

#[tokio::test]
async fn changing_text_root_rejects_startup_and_preserves_current_and_historical_content() {
    let fixture = Fixture::new().await;
    let kind = fixture.kind().await;
    let entity = fixture.entity(&kind, "Storage root fixture").await;
    let id = entity["id"].as_str().unwrap();
    let detail = fixture
        .ok("GET", &format!("/envs/debug/entities/{id}"), Value::Null)
        .await;
    let source = detail["text_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["definition"]["attribute_role"] == "source")
        .unwrap();
    let attribute = source["definition"]["id"].as_str().unwrap();
    let path = format!("/envs/debug/entities/{id}/attributes/{attribute}/text");
    fixture
        .ok(
            "PUT",
            &path,
            json!({"content":"# Updated source","format":"md","expected_revision":1}),
        )
        .await;
    let original = binding(&fixture).await;
    let error = AppState::open(
        &fixture.directory.path().join("ontology.db"),
        &fixture.directory.path().join("empty-files"),
    )
    .await
    .err()
    .expect("a different text root must reject startup");
    assert!(error.to_string().contains("database binding"));
    assert_eq!(binding(&fixture).await, original);

    fixture.state.ready().await.unwrap();
    for (revision, content) in [(1, "# 来源"), (2, "# Updated source")] {
        let (status, body) = fixture
            .call("GET", &format!("{path}/{revision}"), Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["content"], content);
    }
    let reopened = AppState::open(
        &fixture.directory.path().join("ontology.db"),
        &fixture.directory.path().join("files"),
    )
    .await
    .unwrap();
    reopened.ready().await.unwrap();
    assert_eq!(binding(&fixture).await, original);
}

#[tokio::test]
async fn an_empty_database_cannot_rebind_while_another_server_still_owns_its_root() {
    let fixture = Fixture::new().await;
    let original = binding(&fixture).await;
    assert!(AppState::open(
        &fixture.directory.path().join("ontology.db"),
        &fixture.directory.path().join("other-files"),
    )
    .await
    .is_err());
    assert_eq!(binding(&fixture).await, original);
    let kind = fixture.kind().await;
    let entity = fixture.entity(&kind, "After rejected startup").await;
    assert!(!entity["id"].as_str().unwrap().is_empty());
    fixture.state.ready().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_canonical_alias_of_the_same_text_root_can_reopen_the_database() {
    let fixture = Fixture::new().await;
    let alias = fixture.directory.path().join("alias");
    std::os::unix::fs::symlink(fixture.directory.path().join("files"), &alias).unwrap();
    let original = binding(&fixture).await;
    AppState::open(&fixture.directory.path().join("ontology.db"), &alias)
        .await
        .unwrap()
        .ready()
        .await
        .unwrap();
    assert_eq!(binding(&fixture).await, original);
}
