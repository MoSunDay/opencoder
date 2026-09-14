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
    let exe = fleet.root().join("runner.py");
    let source = include_str!("../../dag-runtime/tests/runner/fixture.py").replace("root = pathlib.Path.cwd()", "root = pathlib.Path.cwd() / v['execution_id']\nroot.mkdir(exist_ok=True)")
        .replace("mode = os.environ['MODE'].split(':', 1)[1]", "mode = os.environ['MODE'].split(':', 1)[1]\nif v['input'].get('hold'):\n    import time\n    while not (pathlib.Path.cwd()/'release').exists(): time.sleep(0.02)");
    std::fs::write(&exe, source).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let profile = json!({"executable":exe,"model":"model-v1","auth_slot":1,"envs":{"EXAMPLE":"private-profile-value"}});
    let profile_url = "/api/harnesses/codex/profiles/business";
    assert_eq!(
        fleet.call("PUT", profile_url, profile.clone()).await.body["revision"],
        1
    );
    assert_eq!(
        fleet
            .call("GET", "/api/harnesses/codex/profiles", Value::Null)
            .await
            .body["items"][0]["name"],
        "business"
    );
    let runner = json!({"command":[exe],"workdir":fleet.root(),"envs":{"MODE":"fixture:normal"},"files":{
        exe.display().to_string():opencoder_dag_runtime::exec::runner::artifacts::checksum(&exe).unwrap()}});
    assert_eq!(
        fleet
            .call("PUT", "/api/runners/business", runner.clone())
            .await
            .body["revision"],
        1
    );
    assert_eq!(
        fleet.call("GET", "/api/runners", Value::Null).await.body["items"][0]["name"],
        "business"
    );
    let mut invalid = runner.clone();
    invalid["files"] = json!({});
    assert_eq!(
        fleet
            .call("PUT", "/api/runners/invalid", invalid)
            .await
            .status,
        400
    );
    let spec = json!({"name":"business","steps":[{"name":"diagnose","timeout_secs":20,"kind":{"type":"runner","runner":"business","agent":"act"}}]});
    let reply = fleet
        .call("POST", "/api/dag/defs", json!({"spec":spec}))
        .await;
    assert_eq!(reply.status, 400, "{reply:?}");
    assert!(reply.body.to_string().contains("Runner"), "{reply:?}");
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
    assert!(reply.body.to_string().contains("Runner"), "{reply:?}");
    assert_eq!(client.call_count(), 0);
    fleet.shutdown().await;
}
