use opencoder_todos::{
    domain,
    review::{context, rerun},
    types::*,
};
use serde_json::json;

fn fixture() -> (WorkflowSpec, WorkflowState) {
    let spec: WorkflowSpec = serde_json::from_value(json!({"schema_version":1,"id":"graph",
        "name":"Review","objective":"ship","constraints":["preserve files"],"todos":[
        {"id":"a","title":"A","requirement_background":"background","instructions":"do A","acceptance":{"criteria":"tested"}},
        {"id":"b","title":"B","requirement_background":"background","instructions":"do B","depends_on":["a"],"acceptance":{"criteria":"tested"}},
        {"id":"c","title":"C","requirement_background":"background","instructions":"do C","depends_on":["b"],"acceptance":{"criteria":"tested"}},
        {"id":"other","title":"Other","requirement_background":"background","instructions":"do other","acceptance":{"criteria":"tested"}}
    ]})).unwrap();
    let mut state = domain::initial_state(&spec, "todos-review".into(), "parent".into());
    state.status = WorkflowStatus::Completed;
    for (id, item) in &mut state.todos {
        item.status = TodoStatus::Passed;
        item.attempt = 3;
        item.active_session_id = Some(format!("session-{id}"));
        item.session_history = vec![format!("session-{id}")];
        item.accepted_generation = Some(4);
        item.candidate = Some(Candidate {
            status: CandidateStatus::Candidate,
            summary: "summary".into(),
            result: Some("dependency output".into()),
            verification: "checked".into(),
            evidence_refs: vec!["output.txt".into()],
            recovery_context: RecoveryContext::default(),
        });
    }
    state.milestones.insert("c".into());
    (spec, state)
}

fn request(state: &WorkflowState) -> rerun::RerunRequest {
    rerun::RerunRequest {
        request_id: "rerun-one".into(),
        todo_id: "b".into(),
        reason: "verify changed environment".into(),
        expected_generation: state.generation,
    }
}

#[test]
fn arbitrary_node_rerun_preserves_unrelated_results_and_history() {
    let (spec, state) = fixture();
    let preview = rerun::preview(&spec, &state, "b").unwrap();
    assert_eq!(preview.affected.into_iter().collect::<Vec<_>>(), ["b", "c"]);
    assert!(preview.blockers.is_empty());
    let next = rerun::apply(&spec, state.clone(), &request(&state)).unwrap();
    assert_eq!(next.world_epoch, state.world_epoch + 1);
    assert_eq!(next.generation, state.generation + 1);
    assert_eq!(next.status, WorkflowStatus::Suspended);
    assert_eq!(next.todos["a"].candidate, state.todos["a"].candidate);
    assert_eq!(next.todos["other"].accepted_generation, Some(4));
    for id in ["b", "c"] {
        assert_eq!(next.todos[id].attempt, 0);
        assert!(next.todos[id].candidate.is_none());
        assert_eq!(
            next.todos[id].session_history,
            state.todos[id].session_history
        );
        assert_eq!(next.todos[id].next_context_mode, Some(ContextMode::Fork));
    }
    assert!(!next.milestones.contains("c"));
    assert_eq!(next.todos["b"].status, TodoStatus::Recovering);
    let ctx = context::dispatch_context(&spec, &next, &spec.todos[1], ContextMode::Fork).unwrap();
    assert_eq!(ctx["rerun"]["reason"], "verify changed environment");
    assert_eq!(
        ctx["accepted_dependencies"][0]["result"],
        "dependency output"
    );
}

#[test]
fn rerun_refuses_unaccepted_dependencies_and_live_drivers() {
    let (spec, mut state) = fixture();
    let req = request(&state);
    state.todos.get_mut("a").unwrap().status = TodoStatus::Failed;
    assert_eq!(
        rerun::preview(&spec, &state, "b").unwrap().blockers,
        vec!["a"]
    );
    assert!(rerun::apply(&spec, state.clone(), &req).is_err());
    state.todos.get_mut("a").unwrap().status = TodoStatus::Passed;
    state.active_todo_ids.insert("other".into());
    assert!(rerun::apply(&spec, state.clone(), &req).is_err());
    assert!(rerun::preview(&spec, &state, "missing").is_err());
}

#[test]
fn context_has_dependency_evidence_and_parent_does_not_receive_result_bodies() {
    let (spec, state) = fixture();
    let ctx = context::dispatch_context(&spec, &state, &spec.todos[1], ContextMode::New).unwrap();
    assert_eq!(ctx["accepted_dependencies"][0]["verification"], "checked");
    assert_eq!(
        ctx["accepted_dependencies"][0]["evidence_refs"],
        json!(["output.txt"])
    );
    let prompt = context::focused_prompt(&ctx).unwrap();
    assert!(prompt.contains("dependency output"));
    assert!(prompt.contains("background"));
    let parent = context::scheduling_state(&state).to_string();
    assert!(!parent.contains("dependency output"));
    assert!(parent.contains("summary"));
}

#[test]
fn rerun_validates_id_and_reason() {
    let (_, state) = fixture();
    let mut req = request(&state);
    req.reason = " ".into();
    assert!(req.validate().is_err());
    req.reason = "valid".into();
    req.request_id = "../x".into();
    assert!(req.validate().is_err());
}
