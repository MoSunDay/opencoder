use super::*;

#[tokio::test]
async fn disabled_uploads_preserve_reads_and_delete_protection() {
    let f = Fixture::new().await;
    assert_eq!(f.file("PUT", "existing", "bytes").await.0, 201);
    let path = f.state.workdir.join("opencoder.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["project"]["todo_storage"]["uploads_enabled"] = json!(false);
    std::fs::write(path, config.to_string()).unwrap();
    assert_eq!(f.file("PUT", "new", "new bytes").await.0, 503);
    assert_eq!(f.file("GET", "existing", "").await.1, b"bytes");
    assert_eq!(f.list().await["files"].as_array().unwrap().len(), 1);
    assert_eq!(
        json_call(&f.app, "DELETE", &f.base(), Value::Null).await.0,
        409
    );
}

#[tokio::test]
async fn unavailable_dedicated_owner_never_falls_back_to_local_export() {
    let f = Fixture::new().await;
    let path = f.state.workdir.join("opencoder.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    config["project"]["todo_storage"]["nfs_service_url"] = json!("http://127.0.0.1:1");
    std::fs::write(path, config.to_string()).unwrap();
    assert_eq!(
        json_call(
            &f.app,
            "POST",
            "/api/project/todos/nfs",
            json!({"enabled":true})
        )
        .await
        .0,
        503
    );
    assert_eq!(f.file("PUT", "note", "still writable").await.0, 201);
}
