use super::support::*;
use opencoder_llm::MockChatClient;
use serde_json::json;

#[tokio::test]
async fn rejects_bad_paths_shapes_duplicates_and_oversize_without_publishing() {
    let server = Server::start(MockChatClient::new()).await;
    server.create("alpha", json!({})).await;
    for (cat, files) in [
        ("tools", json!([file("../outside", "bad")])),
        ("tools", json!([file("a//b", "bad")])),
        ("tools", json!([file("a", "one"), file("a", "two")])),
        ("tools", json!([file("a", "one"), file("a/b", "two")])),
        ("prompts", json!([file("soul.md", " \n")])),
        ("skills", json!([file("probe/file.bin", [0, 1])])),
        ("memory", json!([file("memory.md", [255])])),
        ("memory", json!([file("memory.md", [0])])),
        ("memory", json!([file("topics/bad.md", [255])])),
        ("tools", json!([file("big", vec![0; 1536 * 1024 + 1])])),
    ] {
        let view = server.view("alpha", cat).await;
        let (status, error) = server
            .call(
                "PUT",
                &format!("/api/agents/alpha/resources/{cat}"),
                Some(json!({"baseline":view["baseline"],"files":files})),
            )
            .await;
        assert_eq!(status, 400, "{error}");
        assert_eq!(server.view("alpha", cat).await, view);
    }
    let response = server
        .client
        .put(format!("{}/api/agents/alpha/resources/tools", server.url))
        .header("content-type", "application/json")
        .body(" ".repeat(3 * 1024 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 413);
}

#[tokio::test]
async fn read_errors_symlinks_and_failed_writes_never_become_empty_overwrites() {
    let server = Server::start(MockChatClient::new()).await;
    server.create("alpha", json!({})).await;
    let view = server
        .save("alpha", "tools", json!([file("run", "old")]))
        .await;
    let resource = view["baseline"]["resource"].as_str().unwrap();
    let dir = server.root.join("tools").join(resource);
    // Simulate an interrupted/external writer occupying the next immutable version.
    std::fs::write(dir.join("v2"), "occupied").unwrap();
    let request = json!({"baseline":view["baseline"],"files":[file("run","new")]});
    assert_eq!(
        server
            .call(
                "PUT",
                "/api/agents/alpha/resources/tools",
                Some(request.clone())
            )
            .await
            .0,
        409
    );
    assert_eq!(server.view("alpha", "tools").await, view);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        std::fs::remove_file(dir.join("v1/run")).unwrap();
        let outside = server.temp.path().join("outside");
        std::fs::write(&outside, "secret").unwrap();
        symlink(&outside, dir.join("v1/run")).unwrap();
        assert_eq!(
            server
                .call("GET", "/api/agents/alpha/resources/tools", None)
                .await
                .0,
            400
        );
        assert_eq!(
            server
                .call("PUT", "/api/agents/alpha/resources/tools", Some(request))
                .await
                .0,
            400
        );
        assert_eq!(
            server
                .call(
                    "GET",
                    &format!("/api/agents/resources/tools/{resource}/versions/1/files/run"),
                    None
                )
                .await
                .0,
            400
        );
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "secret");
    }
    std::fs::write(dir.join("meta.json"), "broken").unwrap();
    assert_eq!(
        server
            .call("GET", "/api/agents/alpha/resources/tools", None)
            .await
            .0,
        400
    );
}

#[tokio::test]
async fn legacy_put_uses_url_identity_and_checks_reference_baselines() {
    let server = Server::start(MockChatClient::new()).await;
    let body = json!({"name":"shared","files":[file("memory.md","initial")]});
    assert_eq!(
        server
            .call("POST", "/api/agents/resources/memory", Some(body))
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .call(
                "PUT",
                "/api/agents/resources/memory/shared",
                Some(json!({"files":[file("memory.md","second")]}))
            )
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .call(
                "PUT",
                "/api/agents/resources/memory/shared",
                Some(json!({"name":"wrong","files":[]}))
            )
            .await
            .0,
        400
    );
    server.create("alpha", json!({"memory":"shared"})).await;
    let before = server.view("alpha", "memory").await;
    assert_eq!(
        server
            .call("PUT", "/api/agents/alpha", Some(json!({"current":{}})))
            .await
            .0,
        200
    );
    assert_eq!(
        server
            .call(
                "PUT",
                "/api/agents/alpha/resources/memory",
                Some(json!({"baseline":before["baseline"],"files":[file("memory.md","bad")]}))
            )
            .await
            .0,
        409
    );
}

#[tokio::test]
async fn incomplete_shared_history_aborts_fork_without_changing_reference() {
    let server = Server::start(MockChatClient::new()).await;
    server.scoped(|| {
        for text in ["first", "second"] {
            opencoder_agents::save_resource_version(
                "memory",
                "shared",
                &[opencoder_agents::VersionFile {
                    rel_path: "memory.md".into(),
                    bytes: text.as_bytes().to_vec(),
                }],
            )
            .unwrap();
        }
    });
    server.create("alpha", json!({"memory":"shared"})).await;
    let view = server.view("alpha", "memory").await;
    std::fs::remove_dir_all(server.root.join("memory/shared/v1")).unwrap();
    let result = server
        .call(
            "PUT",
            "/api/agents/alpha/resources/memory",
            Some(json!({"baseline":view["baseline"], "files":[file("memory.md", "new")]})),
        )
        .await;
    assert_eq!(result.0, 404);
    assert_eq!(server.view("alpha", "memory").await, view);
    let directories: Vec<_> = std::fs::read_dir(server.root.join("memory"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(directories, ["shared"]);
}
