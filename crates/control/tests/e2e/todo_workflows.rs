//! TODO workflow surface: template/env/tool management (share-dir scoped),
//! template dispatch to a node and workflow views/controls.

use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, SHARE_GATE};

fn spec(name: &str) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "id": format!("wf-{name}"),
        "name": name,
        "objective": "ship it",
        "todos": [{
            "id": "t1", "title": "T1", "requirement_background": "bg",
            "instructions": "do it", "agent": "act",
            "acceptance": {"criteria": "c"},
        }],
        "metadata": {},
    })
}

/// Every test here rewrites the process-global share dir, so they all run
/// under the shared gate (same pattern as the web crate template tests).
async fn scoped() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-todo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    let agents = root.join("agents-root");
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents));
    guard
}

#[tokio::test]
async fn template_env_and_tool_management() {
    let _guard = scoped().await;
    let h = Harness::new().await;

    // Template lifecycle.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "demo", "spec": spec("demo")})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["current"], json!("v1"));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "demo", "spec": spec("demo")})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    let (status, body) = h.req(Method::GET, "/api/todo/templates", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["templates"][0]["name"], json!("demo"));
    let (status, body) = h.req(Method::GET, "/api/todo/templates/demo", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["name"], json!("demo"));

    // todo.json metadata surface: read current, merge-patch description,
    // reject a `current` outside the known versions.
    let (status, body) = h
        .req(Method::GET, "/api/todo/templates/demo/todo.json", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["current"], json!("v1"));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/todo/templates/demo/todo.json",
            Some(json!({"description": "patched"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["template"]["description"], json!("patched"));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/todo/templates/demo/todo.json",
            Some(json!({"description": "x", "current": "v9"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("unknown version v9"),
        "{body}"
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates/demo/new-version",
            Some(json!({"spec": spec("demo"), "note": "v2"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], json!("v2"));

    let (status, body) = h
        .req(
            Method::GET,
            "/api/todo/templates/demo/v2/context.json",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(Method::GET, "/api/todo/templates/demo/v2/env.json", None)
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(Method::DELETE, "/api/todo/templates/demo/v1", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["ok"] == json!(true) || body["deleted"].is_string(),
        "{body}"
    );
    let (status, _) = h
        .req(Method::DELETE, "/api/todo/templates/demo", None)
        .await;
    assert_eq!(status, 200);

    // Environments: CRUD with tool-ref validation.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/envs",
            Some(json!({"name": "dev", "description": "开发"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h.req(Method::GET, "/api/todo/envs", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["envs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["name"] == json!("dev")));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/todo/envs/dev",
            Some(json!({"tools": ["not-a-ref"]})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h.req(Method::DELETE, "/api/todo/envs/dev", None).await;
    assert_eq!(status, 200, "{body}");

    // Tools listing answers the registry shape.
    let (status, body) = h.req(Method::GET, "/api/todo/tools", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["tools"].as_array().is_some(), "{body}");
}

#[tokio::test]
async fn tools_import_copies_from_agents_root() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    let agents = opencoder_core::agent::agents_dir().unwrap();
    let source = agents
        .join("myagent")
        .join("tools")
        .join("v3")
        .join("ffmpeg");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, "#!/bin/sh\necho ok\n").unwrap();

    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/tools/import",
            Some(json!({"agent": "myagent", "version": "v3", "tool": "ffmpeg"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ref"], json!("/agent/tools/v3/ffmpeg"));
    assert_eq!(body["ok"], json!(true));

    // Missing field, non-v<n> version and missing source are rejected
    // before any copy happens.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/tools/import",
            Some(json!({"agent": "myagent", "version": "v3"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/tools/import",
            Some(json!({"agent": "myagent", "version": "3", "tool": "ffmpeg"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/tools/import",
            Some(json!({"agent": "myagent", "version": "v3", "tool": "nope"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn template_dispatch_creates_todos_execution() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    let (status, _) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "demo", "spec": spec("demo")})),
        )
        .await;
    assert_eq!(status, 200);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates/demo/v1/run",
            Some(json!({"id": "todos-run-1"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["workflow_id"], json!("todos-run-1"));
    assert_eq!(body["execution"]["kind"], json!("todos"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // Missing template/version is a clean 400 before any node call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates/nope/v9/run",
            Some(json!({"id": "todos-run-2"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // Workflow views route to the owning node's inspect payload.
    h.node.set_inspect(
        "todos-run-1",
        json!({"execution": {"id": "todos-run-1", "status": "running"},
               "workflow": {"workflow": {"id": "todos-run-1", "status": "running"}, "items": []}}),
    );
    let (status, body) = h.req(Method::GET, "/api/todo/workflows", None).await;
    assert_eq!(status, 200, "{body}");
    let flows = body["workflows"].as_array().unwrap();
    assert!(
        flows.iter().any(|w| w["id"] == json!("todos-run-1")),
        "{body}"
    );
    let (status, body) = h
        .req(Method::GET, "/api/todo/workflows/todos-run-1", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["workflow"]["id"], json!("todos-run-1"));

    h.node.set_command(
        "todos-run-1",
        "interrupt",
        200,
        json!({"id": "todos-run-1", "status": "interrupted"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/workflows/todos-run-1/interrupt",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("interrupted"));
    h.node.set_command(
        "todos-run-1",
        "resume",
        200,
        json!({"id": "todos-run-1", "status": "running"}),
    );
    let (status, body) = h
        .req(Method::POST, "/api/todo/workflows/todos-run-1/resume", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("running"));

    h.node.set_events(
        "todos-run-1",
        vec![json!({"seq": 1, "kind": "status", "data": {"phase": "accepted"}, "ts": 1})],
        true,
    );
    let (status, text) = h.sse_text("/api/todo/workflows/todos-run-1/events").await;
    assert_eq!(status, 200);
    assert!(text.contains("event: status"), "{text}");
}

/// Like `scoped()` but also returns the temp root (env/tool tests need the
/// agents root path for seeding importable tools).
async fn scoped_root() -> (tokio::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-todo-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    let agents = root.join("agents-root");
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents));
    (guard, root)
}

fn err_of(body: &serde_json::Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

fn seeded_tool(path: std::path::PathBuf) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "#!/bin/sh\n").unwrap();
}

#[tokio::test]
async fn env_crud_merge_semantics_and_validation() {
    let (_guard, share) = scoped_root().await;
    let h = Harness::new().await;
    seeded_tool(share.join("agent/tools/v1/fftool"));

    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/envs",
            Some(json!({
        "name": "dev", "description": "d",
        "tools": ["/agent/tools/v1/fftool"], "env_vars": {"A": "1"}})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, b) = h
        .req(Method::POST, "/api/todo/envs", Some(json!({"name": "dev"})))
        .await;
    assert_eq!(s, 409, "{b}");
    let (s, b) = h.req(Method::POST, "/api/todo/envs", Some(json!({}))).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("name"), "{b}");
    let (s, b) = h
        .req(
            Method::POST,
            "/api/todo/envs",
            Some(json!({"name": "../x"})),
        )
        .await;
    assert_eq!(s, 400, "{b}");

    // GET echoes the stored context (name stamped from the dir).
    let (s, b) = h.req(Method::GET, "/api/todo/envs/dev", None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["name"], json!("dev"), "{b}");
    assert_eq!(b["tools"], json!(["/agent/tools/v1/fftool"]), "{b}");
    assert_eq!(b["env_vars"]["A"], json!("1"), "{b}");
    let (s, _) = h.req(Method::GET, "/api/todo/envs/ghost", None).await;
    assert_eq!(s, 404);

    // PUT merges: absent keys keep their stored value.
    let (s, b) = h
        .req(
            Method::PUT,
            "/api/todo/envs/dev",
            Some(json!({"description": "x"})),
        )
        .await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = h.req(Method::GET, "/api/todo/envs/dev", None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["description"], json!("x"), "{b}");
    assert_eq!(b["tools"], json!(["/agent/tools/v1/fftool"]), "{b}");
    assert_eq!(b["env_vars"]["A"], json!("1"), "{b}");

    let (s, b) = h
        .req(
            Method::PUT,
            "/api/todo/envs/dev",
            Some(json!({"tools": [42]})),
        )
        .await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("工具引用必须是字符串"), "{b}");
    let (s, _) = h
        .req(
            Method::PUT,
            "/api/todo/envs/ghost",
            Some(json!({"description": "x"})),
        )
        .await;
    assert_eq!(s, 404);
    let (s, _) = h.req(Method::DELETE, "/api/todo/envs/ghost", None).await;
    assert_eq!(s, 404);
    let (s, _) = h.req(Method::DELETE, "/api/todo/envs/dev", None).await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn tools_union_listing_skips_active_marker() {
    let (_guard, share) = scoped_root().await;
    let h = Harness::new().await;
    let agents = share.join("agents-root");
    seeded_tool(agents.join("myagent/tools/v3/ffmpeg"));
    seeded_tool(agents.join("active/tools/v1/ghost"));
    seeded_tool(agents.join("other/tools/nightly/tool"));
    seeded_tool(share.join("agent/tools/v2/ffmpeg"));

    let (s, b) = h.req(Method::GET, "/api/todo/tools", None).await;
    assert_eq!(s, 200, "{b}");
    let tools = b["tools"].as_array().unwrap();
    let entry = |ref_: &str| {
        tools
            .iter()
            .find(|t| t["ref"] == json!(ref_))
            .unwrap_or_else(|| panic!("missing {ref_}: {b}"))
    };
    let imported = entry("/agent/tools/v3/ffmpeg");
    assert_eq!(imported["source"], json!("importable"), "{b}");
    assert_eq!(imported["agent"], json!("myagent"), "{b}");
    assert_eq!(
        entry("/agent/tools/v2/ffmpeg")["source"],
        json!("share"),
        "{b}"
    );
    // The `active` marker dir and non-`v<n>` versions never surface.
    assert!(
        tools
            .iter()
            .all(|t| t["ref"] != json!("/agent/tools/v1/ghost")),
        "{b}"
    );
    assert!(
        tools
            .iter()
            .all(|t| t["ref"] != json!("/agent/tools/nightly/tool")),
        "{b}"
    );
}

#[tokio::test]
async fn dispatch_pins_env_and_reaches_node() {
    let (_guard, share) = scoped_root().await;
    let h = Harness::new().await;
    seeded_tool(share.join("agent/tools/v3/ffmpeg"));
    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/envs",
            Some(json!({
        "name": "envrun", "tools": ["/agent/tools/v3/ffmpeg"]})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "envrun", "spec": spec("envrun")})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = h
        .req(
            Method::PUT,
            "/api/todo/templates/envrun/v1/env.json",
            Some(json!({"env": "envrun"})),
        )
        .await;
    assert_eq!(s, 200);

    let (s, b) = h
        .req(
            Method::POST,
            "/api/todo/templates/envrun/v1/run",
            Some(json!({"id": "todos-envrun-1"})),
        )
        .await;
    assert_eq!(s, 202, "{b}");
    assert_eq!(b["workflow_id"], json!("todos-envrun-1"), "{b}");
    assert_eq!(b["execution"]["kind"], json!("todos"), "{b}");
    assert_eq!(b["execution"]["node_id"], json!("node-e2e"), "{b}");
    // The snapshot resolved (env tools present) and the Create reached the
    // node, which journaled the execution id. The pinned definition itself
    // lives in `assignment.definition`, which the harness does not expose,
    // so metadata.env/env_tools cannot be asserted from the node side here.
    assert!(h.node.journal_ids().contains(&"todos-envrun-1".to_string()));
}

#[tokio::test]
async fn dispatch_rejects_missing_env_tool_and_tampered_spec() {
    let (_guard, share) = scoped_root().await;
    let h = Harness::new().await;
    // Env creation copies tool refs verbatim; the dispatch gate resolves them.
    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/envs",
            Some(json!({
        "name": "broken", "tools": ["/agent/tools/v9/ghost"]})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "broken", "spec": spec("broken")})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = h
        .req(
            Method::PUT,
            "/api/todo/templates/broken/v1/env.json",
            Some(json!({"env": "broken"})),
        )
        .await;
    assert_eq!(s, 200);
    let (s, b) = h
        .req(
            Method::POST,
            "/api/todo/templates/broken/v1/run",
            Some(json!({"id": "todos-broken-1"})),
        )
        .await;
    assert_eq!(s, 400, "{b}");
    assert!(
        err_of(&b).contains("environment tool missing: /agent/tools/v9/ghost"),
        "{b}"
    );
    assert!(!h.node.journal_ids().contains(&"todos-broken-1".to_string()));

    // A spec tampered straight on the share (bypassing the write API) is
    // caught at dispatch time before any node Create.
    let (s, _) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "tamper", "spec": spec("tamper")})),
        )
        .await;
    assert_eq!(s, 200);
    std::fs::write(
        share.join("todo/tamper/v1/context.json"),
        "{\"todos\":\"x\"}",
    )
    .unwrap();
    let (s, b) = h
        .req(
            Method::POST,
            "/api/todo/templates/tamper/v1/run",
            Some(json!({"id": "todos-tamper-1"})),
        )
        .await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("template:"), "{b}");
    assert!(!h.node.journal_ids().contains(&"todos-tamper-1".to_string()));
}
