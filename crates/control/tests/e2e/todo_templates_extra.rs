//! Extra `/api/todo/templates|envs|tools` e2e coverage: template create
//! validation, meta/context/env-binding contracts, version lifecycle rules,
//! env CRUD edges, the tools union listing and dispatch-time snapshot
//! behaviour (env tool pinning + tampered share specs). Every test rewrites
//! the process-global share/agents overrides, so all run under SHARE_GATE
//! (same serialization as `todo_workflows.rs`).

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, SHARE_GATE};

type Guard = tokio::sync::MutexGuard<'static, ()>;

/// Fresh share + agents roots per test, serialized against the other todo
/// suites by the shared gate; returns the share root for direct seeding.
async fn scoped() -> (Guard, std::path::PathBuf) {
    let guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-tpl2-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    let agents = root.join("agents-root");
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents));
    (guard, root)
}

/// Minimal valid single-TODO WorkflowSpec (unique ids per label).
fn spec(label: &str) -> Value {
    json!({
        "schema_version": 1, "id": format!("wf-{label}"), "name": label,
        "objective": "ship it",
        "todos": [{
            "id": "t1", "title": "T1", "requirement_background": "bg",
            "instructions": "do it", "agent": "act",
            "acceptance": {"criteria": "c"},
        }],
        "metadata": {},
    })
}

/// One-line request helper: keeps the assertions below compact.
async fn http(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    h.req(method, path, body).await
}

fn err_of(body: &Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

/// POST /api/todo/templates with a body (bad-request probes).
async fn create(h: &Harness, body: Value) -> (reqwest::StatusCode, Value) {
    http(h, Method::POST, "/api/todo/templates", Some(body)).await
}

#[tokio::test]
async fn template_create_rejects_invalid_bodies() {
    let _guard = scoped().await.0;
    let h = Harness::new().await;

    let (s, b) = create(&h, json!({})).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("name"), "{b}");
    let (s, b) = create(&h, json!({"name": "../x"})).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("分隔符"), "{b}");
    let (s, b) = create(&h, json!({"name": "d"})).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("spec"), "{b}");

    // Parse failure: the spec must deserialize as a WorkflowSpec.
    let (s, b) = create(&h, json!({"name": "d", "spec": {"todos": "x"}})).await;
    assert_eq!(s, 400, "{b}");

    // Domain validation: empty todo list / self-dependency.
    let mut empty = spec("d");
    empty["todos"] = json!([]);
    let (s, b) = create(&h, json!({"name": "d", "spec": empty})).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("at least one TODO"), "{b}");
    let mut cycle = spec("d");
    cycle["todos"][0]["depends_on"] = json!(["t1"]);
    let (s, b) = create(&h, json!({"name": "d", "spec": cycle})).await;
    assert_eq!(s, 400, "{b}");
    assert!(err_of(&b).contains("invalid dependency"), "{b}");

    // Nothing above may have created the template.
    let (s, _) = http(&h, Method::GET, "/api/todo/templates/d", None).await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn template_meta_and_context_contract() {
    let _guard = scoped().await.0;
    let h = Harness::new().await;
    let (s, _) = http(
        &h,
        Method::POST,
        "/api/todo/templates",
        Some(json!({"name": "meta", "spec": spec("meta")})),
    )
    .await;
    assert_eq!(s, 200);

    let (s, _) = http(&h, Method::GET, "/api/todo/templates/none", None).await;
    assert_eq!(s, 404);
    let (s, _) = http(
        &h,
        Method::PUT,
        "/api/todo/templates/none/todo.json",
        Some(json!({"description": "x"})),
    )
    .await;
    assert_eq!(s, 404);

    // A known version can be flipped back to `current` and it persists.
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/meta/new-version",
        Some(json!({"spec": spec("meta"),"expected_current":"v1"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = http(
        &h,
        Method::PUT,
        "/api/todo/templates/meta/todo.json",
        Some(json!({"current": "v1"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = http(&h, Method::GET, "/api/todo/templates/meta/todo.json", None).await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["template"]["current"], json!("v1"), "{b}");

    // Unknown version context.
    let (s, _) = http(
        &h,
        Method::GET,
        "/api/todo/templates/meta/v9/context.json",
        None,
    )
    .await;
    assert_eq!(s, 404);

    // Existing versions are immutable; edits publish a complete new directory.
    let (s, _) = http(
        &h,
        Method::PUT,
        "/api/todo/templates/meta/v1/context.json",
        Some(spec("meta2")),
    )
    .await;
    assert_eq!(s, 409);
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/meta/new-version",
        Some(json!({"source_version":"v1","expected_current":"v1","spec":spec("meta2")})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["version"], "v3");
    let (_, old) = http(
        &h,
        Method::GET,
        "/api/todo/templates/meta/v1/context.json",
        None,
    )
    .await;
    assert_eq!(old["id"], "wf-meta");
    let mut bad = spec("bad");
    bad["todos"] = json!([]);
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/meta/new-version",
        Some(json!({"source_version":"v3","expected_current":"v3","spec":bad})),
    )
    .await;
    assert_eq!(s, 400, "{b}");
    assert!(b["diagnostics"].is_array());
    let (_, current) = http(
        &h,
        Method::GET,
        "/api/todo/templates/meta/v3/context.json",
        None,
    )
    .await;
    assert_eq!(current["id"], "wf-meta2");
}

#[tokio::test]
async fn env_binding_roundtrip_tombstone_and_map() {
    let _guard = scoped().await.0;
    let h = Harness::new().await;
    assert_eq!(
        create(&h, json!({"name":"bind","spec":spec("bind")}))
            .await
            .0,
        200
    );
    assert_eq!(
        http(
            &h,
            Method::POST,
            "/api/todo/envs",
            Some(json!({"name":"dev"}))
        )
        .await
        .0,
        200
    );
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/bind/new-version",
        Some(json!({"source_version":"v1","expected_current":"v1","binding":{"env":"dev"}})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["version"], "v2");
    let (_, b) = http(
        &h,
        Method::GET,
        "/api/todo/templates/bind/v1/env.json",
        None,
    )
    .await;
    assert!(b["env"].is_null());
    let (_, b) = http(
        &h,
        Method::GET,
        "/api/todo/templates/bind/v2/env.json",
        None,
    )
    .await;
    assert_eq!(b["env"], "dev");
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/bind/new-version",
        Some(json!({"source_version":"v2","expected_current":"v2","binding":{"env":"ghost"}})),
    )
    .await;
    assert_eq!(s, 400, "{b}");
    assert_eq!(b["diagnostics"][0]["path"], "env.json");
    let (s, _) = http(
        &h,
        Method::PUT,
        "/api/todo/templates/bind/v2/env.json",
        Some(json!({"env":null})),
    )
    .await;
    assert_eq!(s, 409);
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/bind/new-version",
        Some(json!({"source_version":"v2","expected_current":"v2","binding":{"env":null}})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let (_, b) = http(&h, Method::GET, "/api/todo/templates/bind", None).await;
    assert_eq!(b["env_by_version"]["v2"], "dev");
    assert!(b["env_by_version"]["v3"].is_null());
}

#[tokio::test]
async fn version_fork_carries_context_and_delete_rules() {
    let _guard = scoped().await.0;
    let h = Harness::new().await;
    let (s, _) = http(
        &h,
        Method::POST,
        "/api/todo/templates",
        Some(json!({"name": "ver", "spec": spec("ver")})),
    )
    .await;
    assert_eq!(s, 200);
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/ver/new-version",
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["version"], json!("v2"), "{b}");

    // Forking from an explicit source copies its context verbatim.
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/ver/new-version",
        Some(json!({"source_version": "v1"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["version"], json!("v3"), "{b}");
    let (s, b) = http(
        &h,
        Method::GET,
        "/api/todo/templates/ver/v3/context.json",
        None,
    )
    .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["id"], json!("wf-ver"), "{b}");

    let (s, _) = http(
        &h,
        Method::POST,
        "/api/todo/templates/none/new-version",
        Some(json!({"source_version": "v1"})),
    )
    .await;
    assert_eq!(s, 404);
    let (s, b) = http(
        &h,
        Method::POST,
        "/api/todo/templates/ver/new-version",
        Some(json!({"source_version": "v9"})),
    )
    .await;
    assert_eq!(s, 404, "{b}");

    // v3 is current after the fork → 409; unknown/misnamed versions → 404/400.
    let (s, b) = http(&h, Method::DELETE, "/api/todo/templates/ver/v3", None).await;
    assert_eq!(s, 409, "{b}");
    assert!(err_of(&b).contains("current"), "{b}");
    let (s, _) = http(&h, Method::DELETE, "/api/todo/templates/ver/v9", None).await;
    assert_eq!(s, 404);
    // `active` is a legal share name but no such version dir exists → 404.
    let (s, _) = http(&h, Method::DELETE, "/api/todo/templates/ver/active", None).await;
    assert_eq!(s, 404);
    // A traversal-shaped version segment (percent-encoded) is a 400.
    let (s, b) = http(&h, Method::DELETE, "/api/todo/templates/ver/a%2Fb", None).await;
    assert_eq!(s, 400, "{b}");
    let (s, _) = http(&h, Method::DELETE, "/api/todo/templates/none", None).await;
    assert_eq!(s, 404);

    // Deleting a non-current version prunes the metadata list.
    let (s, b) = http(&h, Method::DELETE, "/api/todo/templates/ver/v1", None).await;
    assert_eq!(s, 200, "{b}");
    let (s, b) = http(&h, Method::GET, "/api/todo/templates/ver/todo.json", None).await;
    assert_eq!(s, 200, "{b}");
    let listed = b["template"]["versions"].as_array().unwrap().to_vec();
    assert!(listed.iter().all(|v| v["version"] != json!("v1")), "{b}");
}
