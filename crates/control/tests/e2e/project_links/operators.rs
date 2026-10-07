use crate::support::Harness;
use base64::Engine;
use opencoder_core::fleet::RpcReply;
use reqwest::Method;
use serde_json::{json, Value};

async fn save(h: &Harness, path: &str, body: Value) -> Value {
    let (status, reply) = h.req(Method::POST, path, Some(body)).await;
    assert!(status.is_success(), "{path}: {status} {reply}");
    reply
}

#[tokio::test]
async fn six_registered_codex_employees_are_reusable_by_agent_dag_and_brain() {
    let h = Harness::with_brain_kind().await;
    h.node
        .set_capability_reply(RpcReply::ok(json!({"compatible":true,"features":[
            "dag_container_v1","brain_scheduler_v7","brain_contracts_v1"
        ]})));
    let prompt = base64::engine::general_purpose::STANDARD.encode("registered employee v1");
    save(
        &h,
        "/api/agents/resources/prompts",
        json!({"name":"employee","files":[{"path":"soul.md","content_b64":prompt}]}),
    )
    .await;
    for domain in ["harness", "client", "server"] {
        for action in ["root-cause", "code-repair"] {
            let name = format!("{domain}-{action}-operator");
            save(&h, "/api/agents", json!({"name":name,"harness":"codex","harness_profile":"regression-test","run_mode":"operator","current":{"prompt":"employee"}})).await;
            let (_, catalog) = h.req(Method::GET, "/api/brain/library", None).await;
            let cap_id = format!("agent-{name}");
            let cap = catalog["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|cap| cap["id"] == cap_id)
                .unwrap();
            assert_eq!(cap["kind"], "agent");
            assert_eq!(cap["target"], name);
            assert!(cap["definition"].is_object());
            let todo = save(
                &h,
                "/api/project/todos",
                json!({"title":name,"draft":"project work","capability_id":cap_id}),
            )
            .await;
            let path = format!(
                "/api/project/todos/{}/dispatch",
                todo["id"].as_str().unwrap()
            );
            let agent_id = format!("agent-{name}");
            let (status, reply) = h.dispatch(Method::POST, &path, Some(json!({"execution_id":agent_id,"capability_id":cap_id,"input":{"prompt":"individual task"}}))).await;
            assert_eq!(status, 202, "{reply}");
            assert_eq!(h.node.journal_request(&agent_id).unwrap()["target"], name);

            let definition = save(
                &h,
                "/api/dag/defs",
                json!({"name":name,"steps":[{
                    "name":"employee","kind":{"type":"agent","agent":name,"prompt":"workflow task"}
                }]}),
            )
            .await;
            let dag_id = format!("dag-{name}");
            let (status, reply) = h.dispatch(Method::POST, &path, Some(json!({"execution_id":dag_id,"capability_id":format!("dag-{name}"),"input":{"prompt":"workflow"}}))).await;
            assert_eq!(status, 202, "{reply}");
            assert_eq!(
                h.node.pinned_definition(&dag_id).unwrap()["spec"],
                definition["spec"]
            );

            let plan_id = format!("plan-{name}");
            save(&h, "/api/brain/plan-defs", json!({"id":plan_id,"version":1,"changelog":"initial","created_at":1,"plan":{
                "schema_version":7,"title":name,"objective":"complete project work","max_rounds":3,
                "layers":[{"layer_id":"work","title":"work","task":"task","objective":"complete","success_criteria":"evidence"}],
                "nodes":[{"node_id":"employee","layer_id":"work","title":name,"objective":"work","capability_id":cap_id}]
            }})).await;
            let brain_id = format!("brain-{name}");
            let (status, reply) = h.dispatch(Method::POST, &path, Some(json!({"execution_id":brain_id,"capability_id":format!("plan-{plan_id}@1"),"input":{"prompt":"project task"}}))).await;
            assert_eq!(status, 202, "{reply}");
            let brain = h.node.journal_request(&brain_id).unwrap();
            let scope = brain["input"]["capability_scope"].as_array().unwrap();
            assert!(scope.iter().any(|cap| cap["capability_id"] == cap_id
                && cap["kind"] == "agent"
                && cap["target"] == name));
            let (_, meta) = h
                .req(Method::GET, &format!("/api/agents/{name}/meta"), None)
                .await;
            assert_eq!(meta["meta"]["harness"], "codex");
            assert_eq!(meta["meta"]["run_mode"], "operator");
        }
    }
    assert_eq!(h.node.journal_ids().len(), 18);
}
