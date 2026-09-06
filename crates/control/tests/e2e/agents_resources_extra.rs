//! Extra `/api/agents/resources` coverage on the control plane: the full
//! validation matrix (category allowlist, name rules, per-category file
//! shapes, path safety, base64, payload cap), version lifecycle with
//! never-reused numbering and round-trips for the skills/memory/tools
//! pools. Card/active-pointer/NFS/delete-conflict coverage lives in
//! `agents_api.rs`.

use base64::Engine as _;
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, SHARE_GATE};

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

async fn scoped() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(root));
    guard
}

/// One save-body (pure data, no hidden state).
fn save_body(name: &str, path: &str, content: &str) -> Value {
    json!({"name": name, "files": [{"path": path, "content_b64": b64(content)}]})
}

/// POST one save-body into a category; returns (status, body).
async fn post_save(
    h: &Harness,
    cat: &str,
    name: &str,
    path: &str,
    content: &str,
) -> (reqwest::StatusCode, Value) {
    h.req(
        Method::POST,
        &format!("/api/agents/resources/{cat}"),
        Some(save_body(name, path, content)),
    )
    .await
}

/// One request whose status is asserted; returns the parsed body.
async fn want(h: &Harness, method: Method, path: &str, body: Option<Value>, status: u16) -> Value {
    let (got, body) = h.req(method, path, body).await;
    assert_eq!(got.as_u16(), status, "{body}");
    body
}

#[tokio::test]
async fn resource_validation_matrix() {
    let _guard = scoped().await;
    let h = Harness::new().await;

    // Unknown category ⇒ 400 on every route shape that takes :cat.
    for (method, path, body) in [
        ("GET", "/api/agents/resources/nope", None),
        (
            "POST",
            "/api/agents/resources/nope",
            Some(save_body("x", "run.sh", "x")),
        ),
        (
            "PUT",
            "/api/agents/resources/nope/x",
            Some(save_body("x", "run.sh", "x")),
        ),
        ("GET", "/api/agents/resources/nope/x/meta", None),
        (
            "POST",
            "/api/agents/resources/nope/x/rollback",
            Some(json!({"version": 1})),
        ),
        (
            "GET",
            "/api/agents/resources/nope/x/versions/1/files/run.sh",
            None,
        ),
        ("DELETE", "/api/agents/resources/nope/x", None),
    ] {
        let body = want(&h, method.parse().unwrap(), path, body, 400).await;
        assert_eq!(body["ok"], json!(false), "{method} {path}: {body}");
    }

    // Duplicate POST ⇒ 409 (PUT is the version-bump path).
    let (_, body) = post_save(&h, "tools", "dupe-kit", "run.sh", "first").await;
    assert_eq!(body["version"], json!(1), "{body}");
    let (status, body) = post_save(&h, "tools", "dupe-kit", "run.sh", "second").await;
    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("already exists"),
        "{body}"
    );

    // Resource names: charset/length/shape rules (`validate_resource_name`).
    // Whitespace-only trims to empty; reserved agent tokens like `active`
    // are legal resource names — only `.`/`..`/slashes/charset/≤48 apply.
    for bad in [
        "  ",
        ".",
        "..",
        "../x",
        "a/b",
        "a b",
        "中文",
        &"x".repeat(49),
    ] {
        let (status, body) = post_save(&h, "tools", bad, "run.sh", "x").await;
        assert_eq!(status, 400, "name {bad:?}: {body}");
    }

    // Per-category file shapes: prompts exactly the three sections,
    // memory exactly `memory.md`, skills SKILL.md-bearing, tools any
    // safe (incl. nested) path. (cat, path, expect 400?)
    for (cat, path, bad) in [
        ("prompts", "soul.md", false),
        ("prompts", "evil.md", true),
        ("prompts", "nested/soul.md", true),
        ("prompts", "soul.txt", true),
        ("memory", "memory.md", false),
        ("memory", "other.md", true),
        ("memory", "mem/memory.md", true),
        ("skills", "review/SKILL.md", false),
        ("skills", "tips.md", false),
        ("skills", "review/guide.md", true),
        ("skills", "review/SKILL.md/extra.md", true),
        ("skills", "kit.txt", true),
        ("tools", "deep/nested/run.sh", false),
        ("tools", "plain.txt", false),
    ] {
        let (status, body) = post_save(&h, cat, "shape", path, "x").await;
        assert_eq!(status == 400, bad, "{cat}/{path}: {status} {body}");
    }

    // Path safety (checked before any filesystem work): traversal,
    // absolute, empty segments, `.` segments and >64 segments deep.
    let deep = vec!["a"; 65].join("/");
    for path in [
        "../escape",
        "a/../../b",
        "/abs.sh",
        "a//b.md",
        "a/./b.md",
        "",
        &deep,
    ] {
        let (status, body) = post_save(&h, "tools", "safe-kit", path, "x").await;
        assert_eq!(status, 400, "path {path:?}: {body}");
    }

    // Bad base64 ⇒ 400.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/tools",
            Some(json!({"name": "b64-kit", "files": [
                {"path": "run.sh", "content_b64": "@@not-base64@@"},
            ]})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // Decoded payload over the handler's 1.5 MiB cap: base64's 4/3
    // expansion makes any such body exceed 2 MiB on the wire, so the
    // router's default JSON body limit answers 413 before the handler
    // sees it. Assert that outer contract (cap+1 byte decoded).
    let cap = 1536 * 1024;
    let over = json!({
        "name": "cap-kit",
        "files": [{"path": "run.sh", "content_b64": b64(&"a".repeat(cap + 1))}],
    });
    want(
        &h,
        Method::POST,
        "/api/agents/resources/tools",
        Some(over),
        413,
    )
    .await;
    let (_, body) = h
        .req(Method::GET, "/api/agents/resources/tools", None)
        .await;
    assert!(
        !body["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == json!("cap-kit")),
        "{body}"
    );
}

#[tokio::test]
async fn resource_lifecycle_versions_rollback_and_file_fetch() {
    let _guard = scoped().await;
    let h = Harness::new().await;

    // PUT on an unknown resource ⇒ 404 (POST is the create path).
    want(
        &h,
        Method::PUT,
        "/api/agents/resources/prompts/lc-pack",
        Some(save_body("lc-pack", "soul.md", "v1 body")),
        404,
    )
    .await;

    // v1 → v2; pinned version bytes survive later publishes.
    let (_, body) = post_save(&h, "prompts", "lc-pack", "soul.md", "v1 body").await;
    assert_eq!(body["version"], json!(1), "{body}");
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/resources/prompts/lc-pack",
            Some(save_body("lc-pack", "soul.md", "v2 body")),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], json!(2));
    for (version, expected) in [(1, "v1 body"), (2, "v2 body")] {
        let path =
            format!("/api/agents/resources/prompts/lc-pack/versions/{version}/files/soul.md");
        let body = want(&h, Method::GET, &path, None, 200).await;
        assert_eq!(body["content_b64"], json!(b64(expected)), "{body}");
        assert_eq!(body["size"], json!(expected.len()));
    }

    // Rollback to a version outside history ⇒ 400; unknown resource ⇒ 404.
    want(
        &h,
        Method::POST,
        "/api/agents/resources/prompts/lc-pack/rollback",
        Some(json!({"version": 7})),
        400,
    )
    .await;
    want(
        &h,
        Method::POST,
        "/api/agents/resources/prompts/ghost-pack/rollback",
        Some(json!({"version": 1})),
        404,
    )
    .await;

    // Rollback to v1, then PUT: the next version is 3 — numbers are
    // never reused after a rollback.
    let body = want(
        &h,
        Method::POST,
        "/api/agents/resources/prompts/lc-pack/rollback",
        Some(json!({"version": 1})),
        200,
    )
    .await;
    assert_eq!(body, json!({"ok": true, "current": 1}));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/resources/prompts/lc-pack",
            Some(save_body("lc-pack", "soul.md", "v3 body")),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], json!(3), "version numbers never reused");
    let (_, body) = h
        .req(
            Method::GET,
            "/api/agents/resources/prompts/lc-pack/meta",
            None,
        )
        .await;
    assert_eq!(body["meta"]["current"], json!(3), "{body}");
    assert_eq!(body["meta"]["history"], json!([1, 2, 3]), "{body}");

    // File fetch misses: unknown version, unknown resource, missing file
    // inside an existing version ⇒ 404; traversal (`..%2F`) ⇒ 400.
    for path in [
        "/api/agents/resources/prompts/lc-pack/versions/99/files/soul.md",
        "/api/agents/resources/prompts/ghost-pack/versions/1/files/soul.md",
        "/api/agents/resources/prompts/lc-pack/versions/1/files/missing.md",
    ] {
        want(&h, Method::GET, path, None, 404).await;
    }
    want(
        &h,
        Method::GET,
        "/api/agents/resources/prompts/lc-pack/versions/1/files/..%2Fsoul.md",
        None,
        400,
    )
    .await;

    // DELETE of an unknown resource ⇒ 404.
    want(
        &h,
        Method::DELETE,
        "/api/agents/resources/prompts/ghost-pack",
        None,
        404,
    )
    .await;
}

/// The other three pools round-trip end to end: skills take both legal
/// shapes, memory exactly `memory.md`, tools nested paths.
#[tokio::test]
async fn resource_skills_memory_tools_pools_roundtrip() {
    let _guard = scoped().await;
    let h = Harness::new().await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/skills",
            Some(json!({"name": "rev-skills", "files": [
                {"path": "review/SKILL.md", "content_b64": b64("skill v1")},
                {"path": "tips.md", "content_b64": b64("tips v1")},
            ]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], json!(1));
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/resources/skills/rev-skills",
            Some(json!({"name": "rev-skills", "files": [
                {"path": "review/SKILL.md", "content_b64": b64("skill v2")},
            ]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["version"], json!(2));
    // Pinned v1 nested file bytes survive the bump.
    let body = want(
        &h,
        Method::GET,
        "/api/agents/resources/skills/rev-skills/versions/1/files/review/SKILL.md",
        None,
        200,
    )
    .await;
    assert_eq!(body["content_b64"], json!(b64("skill v1")), "{body}");

    let (_, body) = post_save(&h, "memory", "team-mem", "memory.md", "remember this").await;
    assert_eq!(body["version"], json!(1), "{body}");
    let (_, body) = h
        .req(
            Method::GET,
            "/api/agents/resources/memory/team-mem/meta",
            None,
        )
        .await;
    assert_eq!(body["meta"]["current"], json!(1), "{body}");

    let (_, body) = post_save(&h, "tools", "nest-kit", "bin/run.sh", "#!/bin/sh").await;
    assert_eq!(body["version"], json!(1), "{body}");
    let body = want(
        &h,
        Method::GET,
        "/api/agents/resources/tools/nest-kit/versions/1/files/bin/run.sh",
        None,
        200,
    )
    .await;
    assert_eq!(body["path"], json!("bin/run.sh"));
    assert_eq!(body["content_b64"], json!(b64("#!/bin/sh")), "{body}");

    // The list endpoint covers every category.
    for (cat, name) in [
        ("skills", "rev-skills"),
        ("memory", "team-mem"),
        ("tools", "nest-kit"),
    ] {
        let (_, body) = h
            .req(Method::GET, &format!("/api/agents/resources/{cat}"), None)
            .await;
        assert!(
            body["resources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["name"] == json!(name)),
            "{cat}: {body}"
        );
    }
}
