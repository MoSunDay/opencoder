#[path = "graph/support.rs"]
mod support;
use opencoder_brain::{activation, execution as exec, graph};
use opencoder_core::brain::*;
use opencoder_llm::{LlmEvent, MockChatClient};
use serde_json::json;
use support::*;
fn completed(value: serde_json::Value) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: value.to_string(),
        tool_calls: vec![],
        usage: None,
    }]
}
#[tokio::test]
async fn generated_and_handwritten_graphs_have_same_validation_and_execution() {
    let p = fixture();
    let client = MockChatClient::new();
    client.queue_script(completed(json!(p)));
    let mut dynamic=exec::initialize("brain-test",serde_json::from_value(json!({"schema_version":2,"mode":"dynamic","objective":"fixture","inputs":{"document":{"name":"Requirement","markdown":"Fix the issue"}}})).unwrap(),1).unwrap();
    let ctx = exec::context(&mut dynamic);
    let decision = activation::activate(&ctx, &client, "test").await.unwrap();
    exec::adopt(&mut dynamic, version(decision.plan.unwrap()), 2).unwrap();
    let fixed = run(p);
    assert_eq!(dynamic.instances, fixed.instances);
    assert_eq!(dynamic.graph, fixed.graph);
    assert_eq!(client.call_count(), 1);
}
#[tokio::test]
async fn routing_model_receives_only_local_outputs_semantics_and_candidate_inputs() {
    let mut r = run(fixture());
    finish(
        &mut r,
        "fix",
        json!({"fix-result":output("local-content",None)}),
    );
    r.request.objective = "SECRET_GLOBAL_OBJECTIVE".into();
    r.request.inputs.insert(
        "document".into(),
        json!({"name":"requirement","markdown":"SECRET_DOCUMENT"}),
    );
    let ctx = exec::context(&mut r);
    let local = graph::pending(&r)[0].clone();
    let client = MockChatClient::new();
    client.queue_script(completed(
        json!({"receipt":local.receipt,"reason":"repair produced content","selected":["verify"]}),
    ));
    let d = activation::activate(&ctx, &client, "test").await.unwrap();
    assert_eq!(d.routes.len(), 1);
    let wire = client
        .requests()
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.text()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(wire.contains("local-content"));
    assert!(!wire.contains("SECRET_GLOBAL_OBJECTIVE"));
    assert!(!wire.contains("SECRET_DOCUMENT"));
    assert!(!wire.contains("after-verify"));
    graph::apply(&mut r, &d.routes[0], 4).unwrap();
    assert_eq!(r.phase, RunPhase::Running);
}
#[tokio::test]
async fn malformed_route_reply_is_durable_block_not_model_repair_or_agent_fallback() {
    let mut r = run(fixture());
    finish(&mut r, "fix", json!({"fix-result":output("fix",None)}));
    let ctx = exec::context(&mut r);
    let client = MockChatClient::new();
    client.queue_script(vec![LlmEvent::Completed {
        text: "not JSON".into(),
        tool_calls: vec![],
        usage: None,
    }]);
    let d = activation::activate(&ctx, &client, "test").await.unwrap();
    graph::apply(&mut r, &d.routes[0], 4).unwrap();
    assert_eq!(r.phase, RunPhase::Blocked);
    assert!(r.graph.routes.values().all(|r| r.decision.is_some()));
    assert_eq!(client.call_count(), 1);
}
