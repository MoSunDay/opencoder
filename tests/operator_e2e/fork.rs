use crate::support::{
    fleet_proc::Fleet,
    llm_stub::{LlmStub, Script},
};
use serde_json::{json, Value};

#[test]
fn remote_forks_preserve_conversation_type_and_policy_after_restart() {
    for kind in ["operator", "agent"] {
        let probe = |tag: &str| Script::ToolCall {
            name: "bash".into(),
            arguments: json!({"command": format!(
                "printf '%s' \"$HOME\" > fork-home-{tag}.txt; pwd > fork-cwd-{tag}.txt"
            )})
            .to_string(),
        };
        let stub = LlmStub::spawn(vec![
            Script::Text("parent result".into()),
            probe("0"),
            Script::Text("fork result".into()),
            probe("1"),
            Script::Text("restored result".into()),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let mut fleet = Fleet::spawn_conversations(tmp.path(), stub.port(), "fork-node");
        let parent = format!("{kind}-fork-parent");
        let (status, result) = fleet.http("POST", "/api/sessions", &json!({
            "id":parent,"kind":kind,"agent":"act","prompt":"parent prompt", "node_id":fleet.node_id(),
        }));
        assert_eq!(status, 200, "{result}");
        fleet.wait_idle(&parent);
        let parent_home = fleet.node_data.join(kind).join(&parent).join("home");
        if kind == "operator" {
            std::fs::write(
                parent_home.join(".opencoder/skills/fork-frozen.md"),
                "---\nname: fork-frozen\ndescription: Frozen fork test skill\n---\nparent skill\n",
            )
            .unwrap();
        }
        let (status, result) =
            fleet.http("POST", &format!("/api/sessions/{parent}/fork"), &json!({}));
        assert_eq!(status, 200, "{result}");
        let fork = result["id"].as_str().unwrap();
        assert!(fork.starts_with(&format!("{kind}-")), "{fork}");
        for (turn, prompt) in ["fork prompt", "after restart"].into_iter().enumerate() {
            if turn == 1 {
                fleet.respawn_agent();
            }
            // Heartbeats publish the fork before the Server can relay its next turn.
            let idle = fleet.wait_idle(fork);
            assert_eq!(idle["execution"]["kind"], kind);
            assert_eq!(idle["session"]["meta"]["kind"], kind);
            let (status, result) = fleet.http(
                "POST",
                &format!("/api/sessions/{fork}/prompt"),
                &json!({"prompt":prompt,"input_id":format!("fork-turn-{turn}")}),
            );
            assert!(status < 300, "{result}");
            let requests = stub.wait_for_requests(turn * 2 + 3);
            let request: Value = serde_json::from_str(&requests[turn * 2 + 1]).unwrap();
            let system = request["messages"][0]["content"].as_str().unwrap();
            assert_eq!(
                system.matches("## Operator 默认规则").count(),
                usize::from(kind == "operator")
            );
            let skills = fleet
                .node_data
                .join(kind)
                .join(fork)
                .join("home/.opencoder/skills");
            if kind == "operator" {
                assert!(skills.join("task-plan/SKILL.md").is_file());
                assert!(skills.join("fork-frozen.md").is_file());
                assert!(system.contains(skills.canonicalize().unwrap().to_str().unwrap()));
            }
            fleet.wait_idle(fork);
            if kind == "operator" {
                let root = fleet.node_data.join(kind).join(fork);
                for (file, expected) in
                    [("home", root.join("home")), ("cwd", root.join("workspace"))]
                {
                    let actual = std::fs::read_to_string(
                        root.join("workspace")
                            .join(format!("fork-{file}-{turn}.txt")),
                    )
                    .unwrap();
                    assert_eq!(actual.trim(), expected.to_str().unwrap());
                }
                assert!(!parent_home
                    .parent()
                    .unwrap()
                    .join("workspace")
                    .join(format!("fork-home-{turn}.txt"))
                    .exists());
            }
            let (status, page) =
                fleet.http("GET", &format!("/api/sessions?kind={kind}"), &json!({}));
            assert_eq!(status, 200, "{page}");
            assert!(page["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == fork && row["kind"] == kind));
        }
    }
}
