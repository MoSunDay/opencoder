//! The six registered employee identities are Agent cards even when their
//! standalone run_mode is operator. DAG placement must still use runc.
use super::{dispatch, read, save, settled, support, Fleet};
use serde_json::json;

pub async fn verify(fleet: &Fleet) {
    let profile = fleet
        .call(
            "PUT",
            "/api/harnesses/codex/profiles/regression-test",
            json!({"model":"employee-model","reasoning_effort":"high"}),
        )
        .await;
    assert_eq!(profile.status, 200, "{profile:?}");
    let pool = fleet.root().join("n0/node/native-resources/source/agents");
    let mut steps = Vec::new();
    for domain in ["harness", "client", "server"] {
        for action in ["root-cause", "code-repair"] {
            let name = format!("{domain}-{action}-operator");
            std::fs::create_dir_all(pool.join(&name)).unwrap();
            std::fs::write(
                pool.join(&name).join("meta.json"),
                json!({
                    "name":name,"harness":"codex","harness_profile":"regression-test","run_mode":"operator",
                    "current":{"prompt":"probe"}
                })
                .to_string(),
            )
            .unwrap();
            steps.push(
                json!({"name":name,"kind":{"type":"agent","agent":name,"prompt":"CODEX_FIRST"}}),
            );
        }
    }
    save(fleet, json!({"name":"codex-runc","steps":steps})).await;
    let id = "dag-six-registered-operators";
    let accepted = dispatch(fleet, id, json!({})).await;
    assert_eq!(accepted.status, 202, "{accepted:?}");
    let result = settled(&fleet.nodes[0], id).await;
    assert_eq!(result["execution"]["status"], "done", "{result}");
    let root = support::dag_run(&fleet.root().join("n0/node"), id);
    for step in steps {
        let name = step["name"].as_str().unwrap();
        let dir = root.join(name);
        assert_eq!(read(&dir.join("output.json")), json!({"runc":true}));
        let session = read(&dir.join("session.json"));
        assert_eq!(session["harness"], "codex");
        let argv = std::fs::read_to_string(root.join("upper").join(name).join("argv.txt")).unwrap();
        assert!(argv.contains("employee-model"), "{argv}");
        assert!(
            std::fs::read_to_string(root.join("upper").join(name).join("agent-events").join(
                format!("{}.ndjson", session["session_id"].as_str().unwrap())
            ))
            .unwrap()
            .contains("runc-tool-ok")
        );
    }
    assert!(root.join("bundle/config.json").is_file());
    assert!(std::fs::read_dir(root.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
}
