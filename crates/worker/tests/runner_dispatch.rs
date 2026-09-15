#![cfg(unix)]
mod support;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use support::*;

struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        opencoder_core::agent::set_agents_dir_override(None);
    }
}

#[tokio::test]
async fn registered_runner_is_rejected_by_dag_definition_and_inline_dispatch() {
    let client = mock();
    let resource_dir = tempfile::tempdir().unwrap();
    let _config = opencoder_core::scoped_config_home(resource_dir.path().join("config-home"));
    let agents = resource_dir.path().join("agent-source");
    std::fs::create_dir_all(agents.join("act")).unwrap();
    std::fs::write(
        agents.join("act/meta.json"),
        r#"{"name":"act","harness":"codex","harness_profile":"business"}"#,
    )
    .unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents.clone()));
    let _scope = Scope;
    let fleet = Fleet::new(1, client.clone()).await;
    std::fs::write(
        fleet.root().join("n0/work/.opencoder/config.json"),
        r#"{"agent":{"agents_dir":null}}"#,
    )
    .unwrap();
    let exe = fleet.root().join("profile-tool");
    std::fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        fleet
            .call(
                "PUT",
                "/api/harnesses/codex/profiles/business",
                json!({"executable":exe,"model":"model-v1","auth_slot":1})
            )
            .await
            .body["revision"],
        1
    );
    assert!(fleet.call("GET", "/api/runners", Value::Null).await.status >= 400);
    assert!(
        fleet
            .call("PUT", "/api/runners/business", json!({}))
            .await
            .status
            >= 400
    );
    let spec = json!({"name":"business","steps":[{"name":"diagnose","timeout_secs":20,"kind":{"type":"runner","runner":"business","agent":"act"}}]});
    let reply = fleet
        .call("POST", "/api/dag/defs", json!({"spec":spec}))
        .await;
    assert_eq!(reply.status, 400, "{reply:?}");
    assert!(
        reply.body.to_string().contains("unknown variant"),
        "{reply:?}"
    );
    let reply = fleet
        .call(
            "POST",
            "/api/executions",
            json!({
                "id":"dag-runner-rejected", "kind":"dag", "input":{"definition":spec}
            }),
        )
        .await;
    assert_eq!(reply.status, 400, "{reply:?}");
    // Inline dispatch hits the serde boundary first: `runner` is no longer a
    // known step kind variant after the convergence to agent/wasm.
    assert!(
        reply.body.to_string().contains("unknown variant"),
        "{reply:?}"
    );
    assert_eq!(client.call_count(), 0);
    fleet.shutdown().await;
}
