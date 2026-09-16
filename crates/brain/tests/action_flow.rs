#[path = "graph/support.rs"]
mod support;
use opencoder_brain::{execution as exec, graph};
use opencoder_core::brain::*;
use serde_json::json;
use support::*;
#[test]
fn repair_retest_loop_preserves_rounds_and_uses_causal_feedback() {
    let mut r = run(fixture());
    for n in 1..=2 {
        let mut produced = output(&format!("fix-{n}"), None);
        produced["artifacts"] = json!([{
            "execution":{"id":format!("agent-fix-{n}"),"kind":"agent"},
            "step":"fix","file":format!("patch-{n}.diff"),"sha256":"abc","bytes":42
        }]);
        finish(&mut r, "fix", json!({"fix-result":produced}));
        choose(&mut r, "after-fix", &["verify"], None);
        assert_eq!(
            r.instances[&visit(&r, "verify")].inputs["change"],
            format!("fix-{n}")
        );
        finish(
            &mut r,
            "verify",
            json!({"verification":output(if n==1{"still broken"}else{"verified fix-2"},Some(n==2))}),
        );
        let execution = &r.instances[&visit(&r, "verify")]
            .execution
            .as_ref()
            .unwrap()
            .id;
        let sources = &r.actions[execution].request["input"]["source_outputs"];
        assert_eq!(sources.as_object().unwrap().len(), 1);
        let source = &sources["change"];
        assert_eq!(source["round"], n);
        assert_eq!(source["value"], produced);
        assert_eq!(source["id"], format!("fix~visit-{n:04}/fix-result"));
        if n == 1 {
            choose(&mut r, "after-verify", &["fix"], None);
            assert_eq!(
                r.instances[&visit(&r, "fix")].inputs["feedback"],
                "still broken"
            );
        }
    }
    choose(&mut r, "after-verify", &[], Some("done"));
    assert_eq!(r.phase, RunPhase::Completed);
    assert_eq!(r.instances.len(), 4);
    assert_eq!(r.graph.outputs.len(), 4);
    assert_eq!(r.graph.routes.len(), 4);
    assert_eq!(
        r.graph.outputs["fix~visit-0001/fix-result"].value.content,
        "fix-1"
    );
}
#[test]
fn invalid_or_unknown_routing_blocks_and_never_creates_a_fallback() {
    for (selected, exit, verified) in [
        (vec!["foreign"], None, Some(true)),
        (vec![], None, Some(true)),
        (vec![], Some("undeclared"), Some(true)),
        (vec![], Some("done"), None),
        (vec![], Some("done"), Some(false)),
    ] {
        let mut r = run(fixture());
        finish(&mut r, "fix", json!({"fix-result":output("fix",None)}));
        choose(&mut r, "after-fix", &["verify"], None);
        finish(
            &mut r,
            "verify",
            json!({"verification":output("result",verified)}),
        );
        choose(&mut r, "after-verify", &selected, exit);
        assert_eq!(r.phase, RunPhase::Blocked);
        assert_eq!(r.instances.len(), 2);
        assert!(r.error.is_some());
    }
}
#[test]
fn missing_output_and_unsubstantiated_verification_are_not_success() {
    let mut r = run(fixture());
    finish(&mut r, "fix", json!({}));
    assert_eq!(r.phase, RunPhase::Blocked);
    assert!(r.error.unwrap().contains("missing"));
    let mut r = run(fixture());
    finish(&mut r, "fix", json!({"fix-result":output("fix",None)}));
    choose(&mut r, "after-fix", &["verify"], None);
    finish(
        &mut r,
        "verify",
        json!({"verification":{"content":"assertion","verification":{"passed":true,"evidence":[]}}}),
    );
    assert_eq!(
        r.graph.outputs["verify~visit-0001/verification"]
            .value
            .verification
            .passed,
        None
    );
    choose(&mut r, "after-verify", &[], Some("done"));
    assert_eq!(r.phase, RunPhase::Blocked);
}
#[test]
fn replay_pause_cancel_and_visit_limit_are_durable() {
    let mut p = fixture();
    p.instances[0].max_visits = 1;
    let mut r = run(p);
    let n = finish(&mut r, "fix", json!({"fix-result":output("fix",None)}));
    let revision = r.revision;
    assert!(!exec::apply_notice(&mut r, &n).unwrap());
    assert_eq!(revision, r.revision);
    let pending = graph::pending(&r);
    r = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
    assert_eq!(graph::pending(&r), pending);
    let d = choose(&mut r, "after-fix", &["verify"], None);
    let len = r.instances.len();
    graph::apply(&mut r, &d, 5).unwrap();
    assert_eq!(len, r.instances.len());
    finish(
        &mut r,
        "verify",
        json!({"verification":output("broken",Some(false))}),
    );
    choose(&mut r, "after-verify", &["fix"], None);
    assert_eq!(r.phase, RunPhase::Blocked);
    assert!(r.error.unwrap().contains("visit limit"));
    let mut r = run(fixture());
    exec::command(&mut r, "pause", 2).unwrap();
    let id = visit(&r, "fix");
    assert!(exec::prepare(&mut r, &id, "root", 3).is_err());
    exec::command(&mut r, "resume", 3).unwrap();
    exec::prepare(&mut r, &id, "root", 4).unwrap();
    exec::command(&mut r, "cancel", 5).unwrap();
    assert_eq!(r.phase, RunPhase::Cancelling);
    let mut n = notice(&r, &id, json!({}));
    n.status = StepStatus::Cancelled;
    n.output = None;
    exec::apply_notice(&mut r, &n).unwrap();
    assert_eq!(r.phase, RunPhase::Cancelled);
}

#[test]
fn paused_receipts_update_assessments_without_routing_or_dispatch() {
    let mut r = run(fixture());
    let id = visit(&r, "fix");
    exec::prepare(&mut r, &id, "root", 1).unwrap();
    exec::command(&mut r, "pause", 2).unwrap();
    let n = notice(
        &r,
        &id,
        json!({"fix-result":output("changed files",Some(false))}),
    );
    exec::apply_notice(&mut r, &n).unwrap();
    assert_eq!(r.phase, RunPhase::Paused);
    assert_eq!(
        r.graph.outputs[&format!("{id}/fix-result")]
            .value
            .verification
            .passed,
        Some(false)
    );
    assert!(graph::pending(&r).is_empty());
    assert_eq!(r.instances.len(), 1);
    exec::command(&mut r, "resume", 3).unwrap();
    assert_eq!(graph::pending(&r).len(), 1);
}
