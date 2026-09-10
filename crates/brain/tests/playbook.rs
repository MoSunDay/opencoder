//! Pure-domain integration tests for the playbook spec + topology: the
//! validation report (every rule, aggregated), the deterministic Kahn
//! schedule, ready/collapse sets, depth/width metrics, prompt rendering and
//! the serde wire shapes. No runtime, no store — the runtime CRUD over the
//! same domain lives in `playbook_runtime.rs`.

use std::collections::BTreeSet;

use opencoder_brain::playbook::{
    spec, topology, PlaybookInput, PlaybookOrigin, PlaybookSpec, PlaybookStep, PlaybookTarget,
    PlaybookTrigger,
};

fn agent_step(name: &str, deps: &[&str]) -> PlaybookStep {
    PlaybookStep {
        name: name.into(),
        depends_on: deps.iter().map(|d| d.to_string()).collect(),
        target: PlaybookTarget::Agent {
            agent: "act".into(),
        },
        prompt: format!("handle {{situation}} for {name}"),
    }
}

fn spec_with_steps(name: &str, steps: Vec<PlaybookStep>) -> PlaybookSpec {
    PlaybookSpec {
        schema_version: spec::SCHEMA_VERSION,
        id: "playbook-test".into(),
        name: name.into(),
        origin: PlaybookOrigin::Fixed {},
        trigger: PlaybookTrigger::Manual {},
        steps,
    }
}

/// a → b,c → d: the canonical diamond (depth 2, width 2).
fn diamond() -> PlaybookSpec {
    spec_with_steps(
        "diamond",
        vec![
            agent_step("a", &[]),
            agent_step("b", &["a"]),
            agent_step("c", &["a"]),
            agent_step("d", &["b", "c"]),
        ],
    )
}
/// passes, naming the case).
fn rejections(spec_: &PlaybookSpec) -> String {
    match spec::validate(spec_) {
        Err(errs) => errs.join("; "),
        Ok(()) => panic!("spec unexpectedly validated"),
    }
}

#[test]
fn validate_accepts_the_diamond() {
    assert_eq!(spec::validate(&diamond()), Ok(()));
    assert!(spec::is_slug("repro-flake_1"));
    for bad in [
        "",
        "Bad",
        "with space",
        &"x".repeat(spec::MAX_NAME_CHARS + 1),
    ] {
        assert!(!spec::is_slug(bad), "{bad:?} must not be a slug");
    }
}

/// One mutated spec per rule; every expected message must appear in the
/// aggregated report (validate collects ALL problems, not first-only).
#[test]
fn validate_collects_every_rejection_with_its_message() {
    let mutated = |f: &dyn Fn(&mut PlaybookSpec)| {
        let mut s = diamond();
        f(&mut s);
        s
    };
    let too_many: Vec<PlaybookStep> = (0..=spec::MAX_STEPS)
        .map(|i| agent_step(&format!("s{i}"), &[]))
        .collect();
    let deep: Vec<PlaybookStep> = (0..=spec::MAX_CHAIN_DEPTH + 1)
        .map(|i| {
            let prev = format!("s{}", i.saturating_sub(1));
            if i == 0 {
                agent_step("s0", &[])
            } else {
                agent_step(&format!("s{i}"), &[prev.as_str()])
            }
        })
        .collect();
    let wide: Vec<PlaybookStep> = (0..=spec::MAX_WIDTH)
        .map(|i| agent_step(&format!("w{i}"), &[]))
        .collect();
    let mut trigger_bad = diamond();
    trigger_bad.trigger = PlaybookTrigger::Message {
        match_text: "   ".into(),
        threshold: 1.5,
    };

    let cases: Vec<(&str, &str, PlaybookSpec)> = vec![
        (
            "cycle",
            "cycle detected",
            mutated(&|s| s.steps[0].depends_on = vec!["d".into()]),
        ),
        (
            "unknown dep",
            "depends on unknown step",
            mutated(&|s| s.steps[1].depends_on = vec!["zz".into()]),
        ),
        (
            "duplicate name",
            "duplicate step name",
            mutated(&|s| s.steps[1].name = "a".into()),
        ),
        (
            "bad slug",
            "not a valid slug",
            mutated(&|s| s.steps[0].name = "A b".into()),
        ),
        (
            "empty prompt",
            "empty prompt",
            mutated(&|s| s.steps[0].prompt = "   ".into()),
        ),
        (
            "empty target",
            "team must not be empty",
            mutated(&|s| s.steps[0].target = PlaybookTarget::Team { team: "  ".into() }),
        ),
        (
            "too many steps",
            "steps must be <= 64",
            spec_with_steps("m", too_many),
        ),
        (
            "old schema",
            "schema_version must be 1",
            mutated(&|s| s.schema_version = 0),
        ),
        (
            "empty match_text",
            "match_text must not be empty",
            trigger_bad.clone(),
        ),
        ("bad threshold", "threshold must be finite", trigger_bad),
        (
            "self dep",
            "depends on itself",
            mutated(&|s| s.steps[0].depends_on = vec!["a".into()]),
        ),
        (
            "duplicate edge",
            "duplicate dependency",
            mutated(&|s| s.steps[3].depends_on = vec!["b".into(), "b".into(), "c".into()]),
        ),
        (
            "empty id",
            "id must not be empty",
            mutated(&|s| s.id = "  ".into()),
        ),
        (
            "no steps",
            "at least one step",
            mutated(&|s| s.steps = vec![]),
        ),
        (
            "long name",
            "name exceeds",
            mutated(&|s| s.name = "n".repeat(spec::MAX_PLAYBOOK_NAME_CHARS + 1)),
        ),
        (
            "long prompt",
            "prompt exceeds",
            mutated(&|s| s.steps[0].prompt = "x".repeat(spec::MAX_PROMPT_CHARS + 1)),
        ),
        ("too deep", "chain depth", spec_with_steps("deep", deep)),
        ("too wide", "ready width", spec_with_steps("wide", wide)),
    ];
    for (case, needle, bad) in cases {
        let errs = rejections(&bad);
        assert!(
            errs.contains(needle),
            "{case}: expected {needle:?} in {errs:?}"
        );
    }
}

#[test]
fn topo_order_is_deterministic_across_declaration_orders() {
    assert_eq!(
        topology::topo_order(&diamond()).unwrap(),
        vec!["a", "b", "c", "d"]
    );
    let serial = spec_with_steps(
        "serial",
        vec![
            agent_step("a", &[]),
            agent_step("b", &["a"]),
            agent_step("c", &["b"]),
        ],
    );
    assert_eq!(topology::topo_order(&serial).unwrap(), vec!["a", "b", "c"]);
    // Same diamond declared upside-down: levels stay lexicographic.
    let shuffled = spec_with_steps(
        "shuffled",
        vec![
            agent_step("d", &["b", "c"]),
            agent_step("c", &["a"]),
            agent_step("b", &["a"]),
            agent_step("a", &[]),
        ],
    );
    assert_eq!(
        topology::topo_order(&shuffled).unwrap(),
        vec!["a", "b", "c", "d"]
    );
    let cyclic = spec_with_steps(
        "cyclic",
        vec![agent_step("a", &["b"]), agent_step("b", &["a"])],
    );
    assert_eq!(
        topology::topo_order(&cyclic).unwrap_err(),
        "cycle detected involving step \"a\""
    );
    let unknown = spec_with_steps("unknown", vec![agent_step("a", &["zz"])]);
    assert!(topology::topo_order(&unknown)
        .unwrap_err()
        .contains("unknown step"));
}

#[test]
fn ready_steps_follow_the_done_set() {
    let s = diamond();
    let empty: BTreeSet<String> = BTreeSet::new();
    assert_eq!(topology::ready_steps(&s, &empty), vec!["a"]);
    let after_a: BTreeSet<String> = ["a".to_string()].into_iter().collect();
    assert_eq!(topology::ready_steps(&s, &after_a), vec!["b", "c"]);
    // d stays hidden until BOTH b and c are done.
    let after_b: BTreeSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
    assert_eq!(topology::ready_steps(&s, &after_b), vec!["c"]);
    let all: BTreeSet<String> = ["a", "b", "c", "d"]
        .into_iter()
        .map(|n| n.to_string())
        .collect();
    assert!(topology::ready_steps(&s, &all).is_empty());
}

#[test]
fn collapse_blocked_takes_the_transitive_downstream() {
    let s = diamond();
    let failed: BTreeSet<String> = ["b".to_string()].into_iter().collect();
    let blocked = topology::collapse_blocked(&s, &failed);
    let expected: BTreeSet<String> = ["b", "d"].into_iter().map(str::to_string).collect();
    assert_eq!(blocked, expected, "c survives, d collapses with b");
    // Failing the root collapses everything.
    let root: BTreeSet<String> = ["a".to_string()].into_iter().collect();
    assert_eq!(topology::collapse_blocked(&s, &root).len(), 4);
}

#[test]
fn chain_depth_and_max_width_metrics() {
    assert_eq!(topology::chain_depth(&diamond()), 2);
    assert_eq!(topology::max_width(&diamond()), 2);
    let empty = spec_with_steps("empty", vec![]);
    assert_eq!(topology::chain_depth(&empty), 0);
    assert_eq!(topology::max_width(&empty), 0);
    let cyclic = spec_with_steps(
        "cyclic",
        vec![agent_step("a", &["b"]), agent_step("b", &["a"])],
    );
    // steps.len() + 1: past every limit, flagged by the topo error anyway.
    assert_eq!(topology::chain_depth(&cyclic), 3);
    assert_eq!(topology::max_width(&cyclic), 3);
    let wide = spec_with_steps(
        "wide",
        (0..5).map(|i| agent_step(&format!("w{i}"), &[])).collect(),
    );
    assert_eq!(topology::max_width(&wide), 5);
    assert_eq!(topology::chain_depth(&wide), 0);
}

#[test]
fn render_prompt_substitutes_only_the_situation_placeholder() {
    assert_eq!(
        spec::render_prompt("fix {situation} via {plan}", "  flaky test  "),
        "fix flaky test via {plan}"
    );
    assert_eq!(
        spec::render_prompt("no placeholders", "x"),
        "no placeholders"
    );
}

#[test]
fn serde_wire_shapes_roundtrip() {
    let json = r#"{
        "schema_version": 1, "id": "playbook-1", "name": "fix flake",
        "origin": {"kind": "dynamic", "situation_digest": "dig", "plan_id": null},
        "trigger": {"kind": "message", "match_text": "test is flaky", "threshold": 0.82},
        "steps": [
            {"name": "repro", "target": {"kind": "agent", "agent": "act"}, "prompt": "repro {situation}"},
            {"name": "review", "depends_on": ["repro"], "target": {"kind": "team", "team": "core"}, "prompt": "review"},
            {"name": "flow", "depends_on": ["review"], "target": {"kind": "dag", "dag": "etl"}, "prompt": "run"},
            {"name": "todo", "depends_on": ["review"], "target": {"kind": "todos", "workflow": "wf"}, "prompt": "track"},
            {"name": "cap", "depends_on": ["review"], "target": {"kind": "brain", "capability_id": "brain-1"}, "prompt": "use"}
        ]
    }"#;
    let parsed: PlaybookSpec = serde_json::from_str(json).unwrap();
    assert!(matches!(
        parsed.origin,
        PlaybookOrigin::Dynamic { ref situation_digest, plan_id: None } if situation_digest == "dig"
    ));
    assert!(matches!(
        parsed.trigger,
        PlaybookTrigger::Message { ref match_text, threshold }
            if match_text == "test is flaky" && (threshold - 0.82).abs() < 1e-9
    ));
    // depends_on omitted on the first step: serde default = empty.
    assert!(parsed.steps[0].depends_on.is_empty());
    assert!(spec::validate(&parsed).is_ok());

    let wire = serde_json::to_string(&parsed).unwrap();
    assert!(wire.contains(r#""origin":{"kind":"dynamic","situation_digest":"dig"}"#));
    assert!(wire.contains(r#""trigger":{"kind":"message""#));
    assert!(!wire.contains("plan_id"), "None is skipped");
    assert_eq!(serde_json::from_str::<PlaybookSpec>(&wire).unwrap(), parsed);

    // Unit-shaped variants serialize to the bare tag object.
    assert_eq!(
        serde_json::to_value(PlaybookOrigin::Fixed {}).unwrap(),
        serde_json::json!({"kind": "fixed"})
    );
    assert_eq!(
        serde_json::to_value(PlaybookTrigger::Manual {}).unwrap(),
        serde_json::json!({"kind": "manual"})
    );
    assert!(matches!(
        PlaybookTrigger::default(),
        PlaybookTrigger::Manual {}
    ));

    // Input payload: trigger omitted defaults to manual.
    let parsed: PlaybookInput = serde_json::from_str(
        r#"{"name":"n","steps":[{"name":"a","target":{"kind":"agent","agent":"act"},"prompt":"p"}]}"#,
    )
    .unwrap();
    assert!(matches!(parsed.trigger, PlaybookTrigger::Manual {}));
}
