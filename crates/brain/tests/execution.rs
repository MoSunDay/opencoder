#[path = "graph/support.rs"]
mod support;
use opencoder_brain::{execution as exec, graph};
use opencoder_core::brain::*;
use serde_json::{json, Value};
use support::*;
fn parallel() -> OntologyPlan {
    let mut p = serde_json::to_value(fixture()).unwrap();
    let mut sibling = p["instances"][0].clone();
    sibling["id"] = json!("sibling");
    sibling["inputs"] = json!(["document"]);
    sibling["outputs"] = json!(["side-a", "side-b"]);
    let mut join = sibling.clone();
    join["id"] = json!("join");
    join["inputs"] = json!(["joined-main", "joined-side"]);
    join["outputs"] = json!(["delivery"]);
    p["instances"]
        .as_array_mut()
        .unwrap()
        .extend([sibling, join]);
    for name in ["side-a", "side-b", "delivery"] {
        p["outputs"][name] = json!({"description":name});
    }
    for name in ["joined-main", "joined-side"] {
        p["inputs"][name] = json!({"description":name,"source":{"kind":"routed"},"schema":{"type":"string"},"required":true});
    }
    // verify routes either loop locally or select a new instance that joins sibling.
    let mut ready = p["instances"][1].clone();
    ready["id"] = json!("ready");
    ready["inputs"] = json!(["joined-main"]);
    ready["outputs"] = json!(["ready-result"]);
    p["instances"].as_array_mut().unwrap().push(ready);
    p["outputs"]["ready-result"] = json!({"description":"final verified result"});
    p["routes"][1]["targets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"instance":"ready","bindings":{"joined-main":"verification"}}));
    p["routes"][1]["exits"] = json!([]);
    p["routes"].as_array_mut().unwrap().extend([
 json!({"id":"join-route","description":"Combine selected branches","outputs":["ready-result","side-a","side-b"],"targets":[{"instance":"join","bindings":{"joined-main":"ready-result","joined-side":"side-b"}}],"exits":[]}),
 json!({"id":"finish","description":"deliver","outputs":["delivery"],"targets":[],"exits":[{"id":"done","description":"verified delivery","deliverables":["delivery"],"require_verified":true}]})]);
    p["entry"] = json!(["fix", "sibling"]);
    serde_json::from_value(p).unwrap()
}
#[test]
fn parallel_branch_loop_joins_only_final_causal_outputs_and_multiple_ports() {
    let mut r = run(parallel());
    finish(
        &mut r,
        "sibling",
        json!({"side-a":output("side-a",None),"side-b":output("side-b",None)}),
    );
    assert!(graph::pending(&r).is_empty());
    for n in 1..=2 {
        finish(
            &mut r,
            "fix",
            json!({"fix-result":output(&format!("fix-{n}"),None)}),
        );
        choose(&mut r, "after-fix", &["verify"], None);
        finish(
            &mut r,
            "verify",
            json!({"verification":output(&format!("verify-{n}"),Some(n==2))}),
        );
        choose(
            &mut r,
            "after-verify",
            &[if n == 1 { "fix" } else { "ready" }],
            None,
        );
    }
    finish(
        &mut r,
        "ready",
        json!({"ready-result":output("round-2",Some(true))}),
    );
    let ctx = graph::pending(&r);
    assert_eq!(ctx.len(), 1);
    assert_eq!(ctx[0].outputs.len(), 3);
    assert!(!serde_json::to_string(&ctx).unwrap().contains("fix-1"));
    choose(&mut r, "join-route", &["join"], None);
    let i = &r.instances[&visit(&r, "join")];
    assert_eq!(i.inputs["joined-main"], "round-2");
    assert_eq!(i.inputs["joined-side"], "side-b");
    finish(
        &mut r,
        "join",
        json!({"delivery":output("delivery",Some(true))}),
    );
    choose(&mut r, "finish", &[], Some("done"));
    assert_eq!(r.phase, RunPhase::Completed);
}
#[test]
fn local_context_and_unselected_branch_do_not_wait_on_global_history() {
    let mut p = serde_json::to_value(parallel()).unwrap();
    p["entry"] = json!(["fix"]);
    // sibling remains structurally reachable but is never chosen.
    p["routes"][0]["targets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"instance":"sibling","bindings":{}}));
    p["inputs"]["joined-side"]["required"] = json!(false);
    let mut r = run(serde_json::from_value(p).unwrap());
    finish(&mut r, "fix", json!({"fix-result":output("fix",None)}));
    choose(&mut r, "after-fix", &["verify"], None);
    finish(
        &mut r,
        "verify",
        json!({"verification":output("verified",Some(true))}),
    );
    choose(&mut r, "after-verify", &["ready"], None);
    finish(
        &mut r,
        "ready",
        json!({"ready-result":output("ready",Some(true))}),
    );
    assert_eq!(graph::pending(&r)[0].outputs.len(), 1);
    choose(&mut r, "join-route", &["join"], None);
    assert_eq!(r.phase, RunPhase::Running);
    assert!(!r.expansions.contains_key("sibling"));
}
#[test]
fn all_registered_kinds_use_same_receipt_and_out_of_order_notices_cannot_regress() {
    for kind in BUSINESS_KINDS {
        let mut p = fixture();
        p.instances[0].action.kind = kind;
        let mut r = run(p);
        let id = visit(&r, "fix");
        let action = exec::prepare(&mut r, &id, "root", 2).unwrap();
        assert_eq!(
            r.actions[&action].request["kind"],
            serde_json::to_value(kind).unwrap()
        );
        let mut n = notice(&r, &id, json!({"fix-result":output("fix",None)}));
        n.status = StepStatus::Running;
        exec::apply_notice(&mut r, &n).unwrap();
        n.sequence = 2;
        n.status = StepStatus::Queued;
        assert!(!exec::apply_notice(&mut r, &n).unwrap());
        assert_eq!(r.instances[&id].status, StepStatus::Running);
    }
}
#[test]
fn input_waits_are_immutable_and_fixed_versions_cannot_change() {
    let mut r=exec::initialize("brain-input",serde_json::from_value::<BrainRequest>(json!({"schema_version":2,"mode":"fixed","objective":"input","plan":version(fixture())})).unwrap(),1).unwrap();
    assert_eq!(r.phase, RunPhase::WaitingInput);
    assert!(exec::supply_input(&mut r, "document", Value::Null, 2).is_err());
    let doc = json!({"name":"req","markdown":"body"});
    exec::supply_input(&mut r, "document", doc.clone(), 2).unwrap();
    exec::supply_input(&mut r, "document", doc, 3).unwrap();
    assert!(exec::supply_input(
        &mut r,
        "document",
        json!({"name":"changed","markdown":"x"}),
        4
    )
    .is_err());
    assert!(exec::adopt(&mut r, version(fixture()), 4).is_err());
}

#[test]
fn overlapping_parallel_invocations_join_by_cause_not_output_name_or_arrival_order() {
    let instance = |id: &str, output: &str| json!({"id":id,"description":id,"capability_id":"builtin-agent-act","action":{"kind":"agent","target":"act","prompt":id,"output_mode":"json"},"inputs":["document"],"outputs":[output]});
    let split = |id: &str, output: &str| json!({"id":id,"description":"activate both branches","outputs":[output],"targets":[{"instance":"left","bindings":{}},{"instance":"right","bindings":{}}],"exits":[]});
    let plan = serde_json::from_value(json!({
        "schema_version":2,"title":"concurrent cohorts","objective":"separate causal waves",
        "inputs":{"document":{"description":"request","source":{"kind":"external"},"schema":{"type":"object"},"required":true}},
        "instances":[instance("spawn-a","a"),instance("spawn-b","b"),instance("left","l"),instance("right","r")],
        "outputs":{"a":{"description":"a"},"b":{"description":"b"},"l":{"description":"left evidence"},"r":{"description":"right evidence"}},
        "entry":["spawn-a","spawn-b"],
        "routes":[split("split-a","a"),split("split-b","b"),{"id":"join","description":"deliver this invocation","outputs":["l","r"],"targets":[],"exits":[{"id":"done","description":"both verified","deliverables":["l","r"]}]}]
    })).unwrap();
    let mut r = run(plan);
    finish(&mut r, "spawn-a", json!({"a":output("a",Some(true))}));
    choose(&mut r, "split-a", &["left", "right"], None);
    let left_a = visit(&r, "left");
    let right_a = visit(&r, "right");
    finish(&mut r, "spawn-b", json!({"b":output("b",Some(true))}));
    choose(&mut r, "split-b", &["left", "right"], None);
    let left_b = visit(&r, "left");
    let right_b = visit(&r, "right");
    for (id, key, content) in [(&left_a, "l", "left-a"), (&right_b, "r", "right-b")] {
        exec::prepare(&mut r, id, "root", 2).unwrap();
        let n = notice(&r, id, json!({key:output(content,Some(true))}));
        exec::apply_notice(&mut r, &n).unwrap();
    }
    assert!(
        graph::pending(&r).is_empty(),
        "must not pair left-a with right-b"
    );
    exec::prepare(&mut r, &right_a, "root", 2).unwrap();
    let n = notice(&r, &right_a, json!({"r":output("right-a",Some(true))}));
    exec::apply_notice(&mut r, &n).unwrap();
    let ready = graph::pending(&r);
    assert_eq!(ready.len(), 1);
    assert_eq!(
        ready[0]
            .outputs
            .iter()
            .map(|o| o.value.content.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["left-a", "right-a"]
    );
    choose(&mut r, "join", &[], Some("done"));
    assert_eq!(r.phase, RunPhase::Running);
    exec::prepare(&mut r, &left_b, "root", 2).unwrap();
    let n = notice(&r, &left_b, json!({"l":output("left-b",Some(true))}));
    exec::apply_notice(&mut r, &n).unwrap();
    choose(&mut r, "join", &[], Some("done"));
    assert_eq!(r.phase, RunPhase::Completed);
    assert_eq!(r.graph.exits.len(), 2);
    assert_eq!(r.graph.outputs.len(), 6);
}
