#![cfg(unix)]
mod support;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use support::*;

struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        opencoder_core::agent::set_agents_dir_override(None);
    }
}

#[tokio::test]
async fn registered_runner_http_dispatch_pins_profile_and_resources_while_queued() {
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
    let mut profile = json!({"executable":exe,"model":"model-v1","auth_slot":1,"envs":{"EXAMPLE":"private-profile-value"}});
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
    let spec = json!({"name":"business","steps":[{"name":"workflow","timeout_secs":20,"kind":{"type":"runner","runner":"business","agent":"act"}}]});
    assert_eq!(
        fleet
            .call("POST", "/api/dag/defs", json!({"spec":spec}))
            .await
            .status,
        200
    );
    let node = fleet.nodes[0].registration().id;
    assert_eq!(
        fleet
            .call(
                "PUT",
                &format!("/api/nodes/{node}/scheduling"),
                json!({"max_runs":1,"queue_order":"fifo"})
            )
            .await
            .status,
        200
    );
    for (id, hold) in [("dag-hold", true), ("dag-queued", false)] {
        let request = json!({"id":id,"kind":"dag","target":"business","node_id":node,"input":{"hold":hold,"job_id":"job-test","attempt":1}});
        let reply = fleet.call("POST", "/api/executions", request.clone()).await;
        assert_eq!(reply.status, 202, "{reply:?}");
        if hold {
            tokio::time::timeout(Duration::from_secs(10), async {
                while !fleet.root().join(id).join("starts").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        } else {
            assert_eq!(reply.body["status"], "pending");
        }
        assert_eq!(
            fleet.call("POST", "/api/executions", request).await.status,
            202
        );
    }
    profile["model"] = json!("model-v2");
    assert_eq!(
        fleet.call("PUT", profile_url, profile).await.body["revision"],
        2
    );
    std::fs::write(
        agents.join("act/meta.json"),
        r#"{"name":"act","harness":"opencoder"}"#,
    )
    .unwrap();
    assert!(!fleet.root().join("dag-queued/starts").exists());
    std::fs::write(fleet.root().join("release"), "go").unwrap();
    for id in ["dag-hold", "dag-queued"] {
        let detail = settled(&fleet.nodes[0], id).await;
        assert_eq!(detail["execution"]["status"], "done", "{detail}");
        assert_eq!(detail["runners"][0]["verdict"], "block");
        assert_eq!(detail["runners"][0]["configuration"]["profile_revision"], 1);
        assert!(!detail.to_string().contains("private-profile-value"));
        let invocation: Value = serde_json::from_slice(
            &std::fs::read(fleet.root().join(id).join("invocation.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(invocation["codex"]["model"], "model-v1");
        assert_eq!(
            std::fs::read_to_string(fleet.root().join(id).join("starts")).unwrap(),
            "1\n"
        );
        let messages = fleet
            .call(
                "GET",
                &format!("/api/executions/{id}/messages"),
                Value::Null,
            )
            .await;
        assert_eq!(messages.status, 200);
        assert!(support::messages::message_page_text(
            &json!({"session":{"messages":messages.body}})
        )
        .contains("tool result"));
        let response = fleet
            .response(
                "GET",
                &format!(
                    "/api/executions/{id}/artifact?step=workflow&file=artifacts%2Fresult.json"
                ),
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            fleet
                .call(
                    "POST",
                    &format!("/api/executions/{id}/commands"),
                    json!({"action":"annotate","input":{"delivery_status":"failed"}})
                )
                .await
                .status,
            200
        );
        assert_eq!(
            settled(&fleet.nodes[0], id).await["annotations"]["delivery_status"],
            "failed"
        );
    }
    assert_eq!(client.call_count(), 0);
    fleet.shutdown().await;
}
