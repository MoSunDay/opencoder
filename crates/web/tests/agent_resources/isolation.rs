use super::support::*;
use opencoder_core::{agent::*, config::ToolsScope};
use opencoder_llm::MockChatClient;
use serde_json::json;

#[tokio::test]
async fn shared_resources_fork_all_categories_with_history_bytes_and_modes() {
    let server = Server::start(MockChatClient::new()).await;
    for (cat, path, bytes) in [
        ("prompts", "soul.md", b"soul".as_slice()),
        ("skills", "probe/SKILL.md", b"skill"),
        ("tools", "probe", b"#!/bin/sh\necho ORIGINAL"),
        ("memory", "memory.md", b"memory"),
    ] {
        for _ in 0..2 {
            server.scoped(|| {
                opencoder_agents::save_resource_version(
                    cat,
                    "shared",
                    &[opencoder_agents::VersionFile {
                        rel_path: path.into(),
                        bytes: bytes.into(),
                    }],
                )
                .unwrap()
            });
        }
    }
    let attachment = server.root.join("skills/shared/v2/probe/assets/raw.bin");
    std::fs::create_dir_all(attachment.parent().unwrap()).unwrap();
    std::fs::write(&attachment, [0, 255, 1]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            server.root.join("tools/shared/v2/probe"),
            std::fs::Permissions::from_mode(0o751),
        )
        .unwrap();
    }
    let refs = json!({"prompt":"shared","skills":"shared","tools":"shared","memory":"shared"});
    server.create("alpha", refs.clone()).await;
    server.create("beta", refs).await;
    for (cat, path) in [
        ("prompts", "soul.md"),
        ("skills", "probe/SKILL.md"),
        ("tools", "probe"),
        ("memory", "memory.md"),
    ] {
        let before = server.view("beta", cat).await;
        let mut change = file(path, "changed");
        change.as_object_mut().unwrap().remove("mode");
        let after = server.save("alpha", cat, json!([change])).await;
        assert_eq!(after["baseline"]["version"], 3);
        assert_eq!(after["versions"], json!([1, 2, 3]));
        assert_ne!(after["baseline"]["resource"], "shared");
        assert_eq!(server.view("beta", cat).await, before);
        let resource = after["baseline"]["resource"].as_str().unwrap();
        assert_eq!(
            server.scoped(|| read_resource_meta(cat, resource).unwrap().owner_agent),
            Some("alpha".into())
        );
        assert!(server.root.join(cat).join(resource).join("v1").is_dir());
        if cat == "skills" {
            assert_eq!(contents(&after, "probe/assets/raw.bin"), [0, 255, 1]);
        }
        if cat == "tools" {
            assert_eq!(after["files"][0]["mode"], 0o751);
        }
        let field = if cat == "prompts" { "prompt" } else { cat };
        let stolen = server
            .call(
                "PUT",
                "/api/agents/beta",
                Some(json!({"current":{field:resource}})),
            )
            .await;
        assert_eq!(stolen.0, 400);
        assert_eq!(
            server
                .call(
                    "PUT",
                    &format!("/api/agents/resources/{cat}/{resource}"),
                    Some(json!({"files":[file(path,"bypass")]}))
                )
                .await
                .0,
            403
        );
    }
    server.scoped(|| {
        let alpha = tools_paths(ToolsScope::All, Some("alpha"));
        let beta = tools_paths(ToolsScope::All, Some("beta"));
        assert_eq!(alpha.len(), 2);
        assert_eq!(beta, vec![server.root.join("tools/shared/v2")]);
        assert!(alpha[0].to_string_lossy().contains("agent-"));
        assert_eq!(all_tools_dirs(), beta);
    });
}

#[tokio::test]
async fn restores_create_versions_and_stale_or_parallel_saves_conflict() {
    let server = Server::start(MockChatClient::new()).await;
    server.create("alpha", json!({})).await;
    let first = server
        .save("alpha", "memory", json!([file("memory.md", "one")]))
        .await;
    let path = "/api/agents/alpha/resources/memory";
    let body = json!({"baseline":first["baseline"],"files":[file("memory.md","two")]});
    let (a, b) = tokio::join!(
        server.call("PUT", path, Some(body.clone())),
        server.call("PUT", path, Some(body))
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let second = server.view("alpha", "memory").await;
    let restore = json!({"baseline":second["baseline"],"version":1});
    let (status, third) = server
        .call("POST", &format!("{path}/restore"), Some(restore))
        .await;
    assert_eq!(status, 200);
    assert_eq!(third["baseline"]["version"], 3);
    assert_eq!(contents(&third, "memory.md"), b"one");
    assert_eq!(
        server
            .call(
                "POST",
                &format!("{path}/restore"),
                Some(json!({"baseline":first["baseline"],"version":1}))
            )
            .await
            .0,
        409
    );
    let fourth = server
        .save("alpha", "memory", json!([file("memory.md", "four")]))
        .await;
    assert_eq!(fourth["baseline"]["version"], 4);
    assert_eq!(
        fourth["baseline"]["resource"],
        first["baseline"]["resource"]
    );
}

#[tokio::test]
async fn owned_version_edits_refresh_visible_reference_contents() {
    let server = Server::start(MockChatClient::new()).await;
    server.create("alpha", json!({})).await;
    server
        .save("alpha", "skills", json!([file("first.md", "first")]))
        .await;
    server
        .save("alpha", "skills", json!([file("second.md", "second")]))
        .await;
    let (status, card) = server.call("GET", "/api/agents/alpha/meta", None).await;
    assert_eq!(status, 200);
    assert_eq!(
        card["meta"]["references"]["skills"],
        json!(["first", "second"])
    );
}

#[tokio::test]
async fn builtins_show_real_definitions_and_resource_writes_are_forbidden() {
    let server = Server::start(MockChatClient::new()).await;
    let view = server.view("act", "prompts").await;
    assert_eq!(
        view["builtin_prompt"],
        opencoder_core::resolve_agent("act").unwrap().prompt
    );
    assert_eq!(view["read_only"], true);
    let tools = server.view("act", "tools").await;
    assert_eq!(
        tools["tool_filter"]["Allow"],
        json!(["bash", "task", "question"])
    );
    let result = server
        .call(
            "PUT",
            "/api/agents/act/resources/prompts",
            Some(json!({"baseline":view["baseline"],"files":[file("soul.md","override")]})),
        )
        .await;
    assert_eq!(result.0, 403);
    server.scoped(|| {
        opencoder_agents::save_resource_version(
            "memory",
            "shared",
            &[opencoder_agents::VersionFile {
                rel_path: "memory.md".into(),
                bytes: b"builtin memory".to_vec(),
            }],
        )
        .unwrap()
    });
    server.scoped(|| {
        opencoder_agents::update_agent_refs(
            "act",
            AgentRefs {
                memory: Some("shared".into()),
                ..Default::default()
            },
        )
        .unwrap()
    });
    assert_eq!(
        contents(&server.view("act", "memory").await, "memory.md"),
        b"builtin memory"
    );
}

#[tokio::test]
async fn legacy_current_only_metadata_can_be_read_and_forked() {
    let server = Server::start(MockChatClient::new()).await;
    let resource = server.root.join("memory/legacy");
    std::fs::create_dir_all(resource.join("v1")).unwrap();
    std::fs::write(resource.join("v1/memory.md"), "legacy").unwrap();
    std::fs::write(resource.join("meta.json"), r#"{"current":1}"#).unwrap();
    server.create("alpha", json!({"memory":"legacy"})).await;
    assert_eq!(server.view("alpha", "memory").await["versions"], json!([1]));
    let old = server
        .call(
            "GET",
            "/api/agents/resources/memory/legacy/versions/1/files/memory.md",
            None,
        )
        .await;
    assert_eq!(old.0, 200);
    let saved = server
        .save("alpha", "memory", json!([file("memory.md", "updated")]))
        .await;
    assert_eq!(saved["versions"], json!([1, 2]));
    assert_eq!(
        std::fs::read_to_string(resource.join("v1/memory.md")).unwrap(),
        "legacy"
    );
}
