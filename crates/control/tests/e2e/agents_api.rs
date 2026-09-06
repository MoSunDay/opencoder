//! Versioned-agent surface on the control plane: agent cards (PUT
//! history/references, delete-conflict with referenced pools), active
//! pointer (deactivate/blank/idempotent/preflight semantics) and the NFS
//! export lifecycle. The resource-pool validation matrix and version
//! lifecycle live in `agents_resources_extra.rs`.

use base64::Engine as _;
use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, SHARE_GATE};

fn b64(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

/// One resource save-body (pure data).
fn save_body(name: &str, path: &str, content: &str) -> serde_json::Value {
    json!({"name": name, "files": [{"path": path, "content_b64": b64(content)}]})
}

async fn scoped() -> tokio::sync::MutexGuard<'static, ()> {
    let (_, guard) = scoped_root().await;
    guard
}

/// [`scoped`] but handing back the fresh agents root (the NFS lifecycle
/// test pins the export root via the harness workdir's `opencoder.json`).
async fn scoped_root() -> (std::path::PathBuf, tokio::sync::MutexGuard<'static, ()>) {
    let guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(root.clone()));
    (root, guard)
}

#[tokio::test]
async fn agent_card_lifecycle_and_active_pointer() {
    let _guard = scoped().await;
    let h = Harness::new().await;

    let (status, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["active"], json!(null));

    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name": "alpha", "current": {"prompt": "pack"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["ok"], json!(true));
    let (status, body) = h
        .req(Method::POST, "/api/agents", Some(json!({"name": "alpha"})))
        .await;
    assert_eq!(status, 409, "{body}");
    for bad in ["active", "prompts", "../x", " "] {
        let (status, _) = h
            .req(Method::POST, "/api/agents", Some(json!({"name": bad})))
            .await;
        assert_eq!(status, 400, "{bad}");
    }

    let (status, body) = h.req(Method::GET, "/api/agents/alpha/meta", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["meta"]["name"], json!("alpha"));
    assert_eq!(body["meta"]["current"]["prompt"], json!("pack"));
    let (status, _) = h.req(Method::GET, "/api/agents/ghost/meta", None).await;
    assert_eq!(status, 404);

    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/alpha",
            Some(json!({"current": {"prompt": "pack2"}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // Activation requires the referenced prompts/<name> pool to exist
    // with a live version, so publish pack2 first.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/prompts",
            Some(json!({"name": "pack2", "files": [{"path": "soul.md", "content_b64": b64("v2 pack body")}]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "alpha"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["active"], json!("alpha"));
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "ghost"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = h.req(Method::DELETE, "/api/agents/alpha", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!("alpha"));
    let (status, _) = h.req(Method::DELETE, "/api/agents/alpha", None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn resource_pool_versioning_and_rollback() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    let save = |name: &str, content: &str| json!({"name": name, "files": [{"path": "soul.md", "content_b64": b64(content)}]});
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/prompts",
            Some(save("pack", "be kind")),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    // Second publish bumps to v2 (PUT to the named pool resource).
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/resources/prompts/pack",
            Some(save("pack", "be terse")),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(Method::GET, "/api/agents/resources/prompts", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == json!("pack")),
        "{body}"
    );

    let (status, body) = h
        .req(Method::GET, "/api/agents/resources/prompts/pack/meta", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["meta"]["current"], json!(2));
    assert_eq!(body["meta"]["history"], json!([1, 2]));

    let (status, body) = h
        .req(
            Method::GET,
            "/api/agents/resources/prompts/pack/versions/2/files/soul.md",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["path"], json!("soul.md"));
    assert_eq!(body["content_b64"], json!(b64("be terse")), "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/prompts/pack/rollback",
            Some(json!({"version": 1})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = h
        .req(Method::GET, "/api/agents/resources/prompts/pack/meta", None)
        .await;
    assert_eq!(body["meta"]["current"], json!(1), "{body}");

    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/resources/prompts/pack",
            Some(save("pack", "v3 body")),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(Method::DELETE, "/api/agents/resources/prompts/pack", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = h
        .req(Method::GET, "/api/agents/resources/prompts/pack/meta", None)
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn nfs_status_reports_idle_export_state() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    // Status-only probe: starting a real NFS listener in e2e adds flaky
    // surface without adding router coverage (start/stop share the route).
    let (status, body) = h.req(Method::GET, "/api/agents/nfs", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["status"]["running"], json!(false));
}

/// Publish a minimal prompts pool (one live version) so a card referencing
/// it passes the activation preflight.
async fn publish_prompt_pool(h: &Harness, name: &str, body: &str) {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/prompts",
            Some(json!({"name": name, "files": [{"path": "soul.md", "content_b64": b64(body)}]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

/// PATCH /api/agents/active semantics beyond the happy path: repeat
/// activation answers `{"unchanged": true}` (exact shape), `null`
/// deactivates, and a blank name is a 400 — not a deactivation.
#[tokio::test]
async fn active_pointer_deactivate_blank_and_idempotent_repatch() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    publish_prompt_pool(&h, "drain-pack", "be steady").await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name": "steady", "current": {"prompt": "drain-pack"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "steady"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"ok": true, "active": "steady"}));

    // Same value again ⇒ `unchanged`, no marker churn (exact response).
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "steady"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({"ok": true, "active": "steady", "unchanged": true})
    );
    let (_, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(body["active"], json!("steady"));

    // `null` deactivates.
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": null})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"ok": true, "active": null}));
    let (_, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(body["active"], json!(null));

    // Blank (whitespace-only) is a 400, never a silent deactivation.
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["ok"], json!(false));
    assert!(
        body["error"].as_str().unwrap_or_default().contains("blank"),
        "{body}"
    );
    let (_, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(body["active"], json!(null));
}

/// Activation preflight: a promptless card and a card whose prompt pool
/// was never published are both rejected with 400 BEFORE the marker
/// settles — the previously active agent survives the failed switch.
#[tokio::test]
async fn active_pointer_preflight_rejects_promptless_and_unpublished_cards() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    publish_prompt_pool(&h, "live-pack", "live body").await;
    for (name, current) in [
        ("barebone", json!({})),
        ("dangling", json!({"prompt": "never-published-pack"})),
    ] {
        let (status, body) = h
            .req(
                Method::POST,
                "/api/agents",
                Some(json!({"name": name, "current": current})),
            )
            .await;
        assert_eq!(status, 201, "{name}: {body}");
    }

    // Anchor a good active agent first so rollback is observable.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name": "anchored", "current": {"prompt": "live-pack"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "anchored"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "barebone"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no prompt reference"),
        "{body}"
    );

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/agents/active",
            Some(json!({"active": "dangling"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no live version"),
        "{body}"
    );

    // Marker rolled back: the previous agent stays active.
    let (_, body) = h.req(Method::GET, "/api/agents", None).await;
    assert_eq!(body["active"], json!("anchored"), "{body}");
}

/// PUT /api/agents/:name rewrites the card: unknown agent ⇒ 404; each
/// changed field appends one `{at, field, from, to}` history entry
/// (unchanged fields append nothing) and the `references` snapshot
/// refreshes from the pools' current versions.
#[tokio::test]
async fn agent_card_put_history_and_references_snapshot() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    publish_prompt_pool(&h, "hist-pack", "prompt body").await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/skills",
            Some(json!({"name": "hist-skills", "files": [
                {"path": "review/SKILL.md", "content_b64": b64("skill body")},
            ]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/ghost",
            Some(json!({"current": {}})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name": "historian", "current": {"prompt": "hist-pack"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let (_, body) = h.req(Method::GET, "/api/agents/historian/meta", None).await;
    assert_eq!(body["meta"]["history"], json!([]), "{body}");
    assert_eq!(body["meta"]["references"]["prompt_files"], json!(["soul"]));

    // Add the skills reference: exactly one history entry (skills moves
    // null → hist-skills) and the skills snapshot fills in.
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/agents/historian",
            Some(json!({"current": {"prompt": "hist-pack", "skills": "hist-skills"}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"ok": true}));
    let (_, body) = h.req(Method::GET, "/api/agents/historian/meta", None).await;
    let history = body["meta"]["history"].as_array().cloned().unwrap();
    assert_eq!(history.len(), 1, "{body}");
    assert_eq!(history[0]["field"], json!("skills"));
    assert_eq!(history[0]["from"], json!(null));
    assert_eq!(history[0]["to"], json!("hist-skills"));
    assert!(!history[0]["at"].as_str().unwrap_or_default().is_empty());
    assert_eq!(body["meta"]["references"]["skills"], json!(["review"]));
    assert_eq!(body["meta"]["references"]["prompt_files"], json!(["soul"]));

    // Unchanged refs ⇒ no new history entries.
    let (status, _) = h
        .req(
            Method::PUT,
            "/api/agents/historian",
            Some(json!({"current": {"prompt": "hist-pack", "skills": "hist-skills"}})),
        )
        .await;
    assert_eq!(status, 200);
    let (_, body) = h.req(Method::GET, "/api/agents/historian/meta", None).await;
    assert_eq!(
        body["meta"]["history"].as_array().unwrap().len(),
        1,
        "{body}"
    );

    // Dropping the skills reference appends the reverse entry.
    let (status, _) = h
        .req(
            Method::PUT,
            "/api/agents/historian",
            Some(json!({"current": {"prompt": "hist-pack"}})),
        )
        .await;
    assert_eq!(status, 200);
    let (_, body) = h.req(Method::GET, "/api/agents/historian/meta", None).await;
    let history = body["meta"]["history"].as_array().cloned().unwrap();
    assert_eq!(history.len(), 2, "{body}");
    assert_eq!(history[1]["field"], json!("skills"));
    assert_eq!(history[1]["from"], json!("hist-skills"));
    assert_eq!(history[1]["to"], json!(null));
    assert_eq!(body["meta"]["references"]["skills"], json!([]));
}

/// POST /api/agents/nfs lifecycle: start (ephemeral port + export root
/// from the harness workdir's `opencoder.json`), idempotent reuse
/// (started:false, same port), stop (running:false) and idempotent stop.
/// Serialized against the status-only test above via SHARE_GATE and left
/// stopped, so the idle assertion stays valid in any order.
#[tokio::test]
async fn nfs_lifecycle_start_reuse_and_stop() {
    let (root, _guard) = scoped_root().await;
    let h = Harness::new().await;
    std::fs::create_dir_all(&h.state.workdir).unwrap();
    std::fs::write(
        h.state.workdir.join("opencoder.json"),
        json!({"agent": {"agents_dir": root, "nfs": {"port": 0}}}).to_string(),
    )
    .unwrap();

    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/nfs",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["started"], json!(true));
    assert_eq!(body["status"]["running"], json!(true));
    assert_eq!(body["status"]["host"], json!("127.0.0.1"));
    assert_eq!(body["status"]["read_only"], json!(true));
    let port = body["status"]["port"].as_u64().unwrap();
    assert!(port > 0, "ephemeral port must resolve: {body}");
    assert_eq!(body["status"]["export_root"], json!(root));

    // Second enabled POST reuses the live server: not respawned.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/nfs",
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["started"], json!(false));
    assert_eq!(body["status"]["port"], json!(port));
    assert_eq!(body["status"]["running"], json!(true));

    // Full status shape on GET while running.
    let (status, body) = h.req(Method::GET, "/api/agents/nfs", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"]["port"], json!(port));
    assert_eq!(body["status"]["export_root"], json!(root));

    // Stop ⇒ documented stopped defaults, idempotent.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/nfs",
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["started"], json!(false));
    assert_eq!(body["status"]["running"], json!(false));
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/nfs",
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["started"], json!(false));
    assert_eq!(body["status"]["running"], json!(false));
    let (status, body) = h.req(Method::GET, "/api/agents/nfs", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["status"],
        json!({
            "running": false,
            "host": "127.0.0.1",
            "port": 2049,
            "read_only": true,
            "export_root": "",
        })
    );
}
#[tokio::test]
async fn resource_delete_conflicts_while_referenced_by_card() {
    let _guard = scoped().await;
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents/resources/prompts",
            Some(save_body("shared-pack", "soul.md", "body")),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/agents",
            Some(json!({"name": "card-holder", "current": {"prompt": "shared-pack"}})),
        )
        .await;
    assert_eq!(status, 201, "{body}");

    // Deleting a referenced pool would break the card ⇒ 409 with the
    // referencing card names.
    let (status, body) = h
        .req(
            Method::DELETE,
            "/api/agents/resources/prompts/shared-pack",
            None,
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["referenced_by"], json!(["card-holder"]));
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("referenced"),
        "{body}"
    );
    let (_, body) = h
        .req(
            Method::GET,
            "/api/agents/resources/prompts/shared-pack/meta",
            None,
        )
        .await;
    assert_eq!(
        body["meta"]["current"],
        json!(1),
        "pool untouched by the 409"
    );

    // Once no card references it, the delete goes through.
    let (status, _) = h
        .req(
            Method::PUT,
            "/api/agents/card-holder",
            Some(json!({"current": {}})),
        )
        .await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::DELETE,
            "/api/agents/resources/prompts/shared-pack",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!("shared-pack"));
}
