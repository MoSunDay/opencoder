//! Pure scheduler decisions over a [`DagSpec`]: validation, topological
//! order, the ready-set, upstream context rendering, and run-level
//! convergence. No IO, no clocks — the runtime feeds these functions state
//! snapshots and acts on the answers.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::artifacts::validate_step_slug;
use crate::spec::{DagSpec, StepKind};
use crate::transitions::{DagRunStatus, StepOutcome};

/// Terminal outcome per step name (the runtime's fold of `step_done` events).
pub type StepStates = BTreeMap<String, StepOutcome>;

/// Parsed upstream artifacts per step name (`output.json` decoded; `null`
/// when the step wrote none).
pub type StepOutputs = BTreeMap<String, Value>;

/// Validate a spec: non-empty name, ≥1 step, slug-legal unique names,
/// deps that exist (no self/duplicate edges), and acyclic. Returns every
/// problem found (aggregated, not first-only) so dispatch rejects with a
/// complete report.
pub fn validate(spec: &DagSpec) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let name_len = spec.name.trim().chars().count();
    if name_len == 0 || name_len > 100 {
        errs.push(format!("spec.name must be 1..=100 chars, got {name_len}"));
    }
    if spec.steps.is_empty() {
        errs.push("spec.steps must not be empty".to_string());
    }
    if spec.steps.len() > 64 {
        errs.push(format!(
            "spec.steps must be <= 64, got {}",
            spec.steps.len()
        ));
    }
    for step in &spec.steps {
        if !validate_step_slug(&step.name) {
            errs.push(format!("step name {:?} is not a valid slug", step.name));
        }
        match &step.kind {
            StepKind::Agent { prompt, .. } if prompt.trim().is_empty() => {
                errs.push(format!("agent step {:?} has an empty prompt", step.name));
            }
            StepKind::Python { code, .. } if code.trim().is_empty() => {
                errs.push(format!("python step {:?} has empty code", step.name));
            }
            _ => {}
        }
    }
    let names: Vec<&str> = spec.steps.iter().map(|s| s.name.as_str()).collect();
    for (i, n) in names.iter().enumerate() {
        if names[..i].contains(n) {
            errs.push(format!("duplicate step name {n:?}"));
        }
    }
    for step in &spec.steps {
        for dep in &step.depends_on {
            if dep == &step.name {
                errs.push(format!("step {:?} depends on itself", step.name));
            } else if !names.contains(&dep.as_str()) {
                errs.push(format!(
                    "step {:?} depends on unknown step {dep:?}",
                    step.name
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for dep in &step.depends_on {
            if !seen.insert(dep) {
                errs.push(format!(
                    "step {:?} lists dependency {dep:?} twice",
                    step.name
                ));
            }
        }
    }
    if let Err(cycle) = topo_order(spec) {
        errs.push(cycle);
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Deterministic topological order (Kahn with the spec's own step order as
/// the frontier tiebreak). `Err` names the cycle.
pub fn topo_order(spec: &DagSpec) -> Result<Vec<String>, String> {
    let mut indegree: BTreeMap<&str, usize> = spec
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s.depends_on.len()))
        .collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for step in &spec.steps {
        for dep in &step.depends_on {
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(step.name.as_str());
        }
    }
    let mut order = Vec::with_capacity(spec.steps.len());
    // Frontier seeded and re-seeded in SPEC order so output is stable.
    loop {
        let before = order.len();
        for step in &spec.steps {
            let d = indegree.get(step.name.as_str()).copied().unwrap_or(0);
            if d == 0 && !order.iter().any(|n| n == &step.name) {
                order.push(step.name.clone());
            }
        }
        if order.len() == before {
            break;
        }
        // Peel one layer of edges.
        let layer: Vec<String> = order[before..].to_vec();
        for n in &layer {
            if let Some(deps) = dependents.get(n.as_str()) {
                for d in deps {
                    if let Some(slot) = indegree.get_mut(d) {
                        *slot = slot.saturating_sub(1);
                    }
                }
            }
        }
        if order.len() == spec.steps.len() {
            return Ok(order);
        }
    }
    let missing: Vec<&str> = indegree
        .iter()
        .filter(|(_, d)| **d > 0)
        .map(|(n, _)| *n)
        .collect();
    Err(format!(
        "cycle detected among steps: {}",
        missing.join(", ")
    ))
}

/// Steps whose dependencies are all `Done` and which have no terminal
/// outcome yet, in spec order. The runtime's entire scheduling decision.
pub fn ready_steps(spec: &DagSpec, states: &StepStates) -> Vec<String> {
    spec.steps
        .iter()
        .filter(|s| !states.contains_key(&s.name))
        .filter(|s| {
            s.depends_on
                .iter()
                .all(|d| states.get(d) == Some(&StepOutcome::Done))
        })
        .map(|s| s.name.clone())
        .collect()
}

/// Fold the run-level terminal status: all steps `Done` -> `done`; any
/// step terminal-failed while nothing is runnable -> `error`/`cancelled`
/// (cancelled wins, mirroring the node executor's terminal precedence);
/// otherwise the run is still live (`None`).
pub fn run_outcome(spec: &DagSpec, states: &StepStates) -> Option<DagRunStatus> {
    if states.is_empty() || states.len() < spec.steps.len() {
        return if ready_steps(spec, states).is_empty()
            && spec.steps.iter().all(|s| states.contains_key(&s.name))
        {
            // All steps terminal but at least one failed.
            terminal_fold(states)
        } else {
            None
        };
    }
    if states.values().all(|o| o == &StepOutcome::Done) {
        return Some(DagRunStatus::Done);
    }
    terminal_fold(states)
}

/// Terminal fold over a COMPLETE map (every step has an outcome): cancelled
/// beats error, error beats done.
fn terminal_fold(states: &StepStates) -> Option<DagRunStatus> {
    if states.values().any(|o| o == &StepOutcome::Cancelled) {
        Some(DagRunStatus::Cancelled)
    } else if states.values().any(|o| o == &StepOutcome::Error) {
        Some(DagRunStatus::Error)
    } else {
        Some(DagRunStatus::Done)
    }
}

/// Build the `context` object injected into python steps (VM global
/// `context` / container `/workspace/context` view of upstream results):
/// `{"steps": {"<name>": {"json": <output.json | null>, "ok": bool}}}`.
/// Only DIRECT and transitive upstream steps of `step` are included, so a
/// step never observes siblings it did not declare a path to.
pub fn render_context(
    spec: &DagSpec,
    step: &str,
    states: &StepStates,
    outputs: &StepOutputs,
) -> Value {
    let upstream = upstream_of(spec, step);
    let mut steps = serde_json::Map::new();
    for name in upstream {
        let ok = states.get(&name).map(|o| o.is_success()).unwrap_or(false);
        let json = outputs.get(&name).cloned().unwrap_or(Value::Null);
        steps.insert(name, json!({ "ok": ok, "json": json }));
    }
    json!({ "steps": Value::Object(steps) })
}

/// All direct+transitive dependencies of `step` (spec order, `step` itself
/// excluded). Unknown `step` yields an empty set — validation already
/// rejects that case at dispatch.
pub fn upstream_of(spec: &DagSpec, step: &str) -> Vec<String> {
    let by_name: BTreeMap<&str, &Vec<String>> = spec
        .steps
        .iter()
        .map(|s| (s.name.as_str(), &s.depends_on))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut stack: Vec<String> = by_name
        .get(step)
        .map(|deps| (*deps).clone())
        .unwrap_or_default();
    while let Some(n) = stack.pop() {
        if seen.insert(n.clone()) {
            if let Some(deps) = by_name.get(n.as_str()) {
                stack.extend(deps.iter().cloned());
            }
        }
    }
    spec.steps
        .iter()
        .filter(|s| seen.contains(&s.name))
        .map(|s| s.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec_from(v: Value) -> DagSpec {
        serde_json::from_value(v).unwrap()
    }

    fn states(pairs: &[(&str, StepOutcome)]) -> StepStates {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn validate_reports_every_problem() {
        let spec = spec_from(json!({
            "name": "",
            "steps": [
                { "name": "Bad Name", "kind": { "type": "agent", "prompt": " " } },
                { "name": "b", "depends_on": ["b", "missing", "a", "missing"], "kind": { "type": "python", "code": "" } }
            ]
        }));
        let errs = validate(&spec).unwrap_err();
        let joined = errs.join("; ");
        assert!(joined.contains("spec.name"), "{joined}");
        assert!(joined.contains("Bad Name"), "{joined}");
        assert!(joined.contains("empty prompt"), "{joined}");
        assert!(joined.contains("empty code"), "{joined}");
        assert!(joined.contains("depends on itself"), "{joined}");
        assert!(joined.contains("unknown step \"missing\""), "{joined}");
        assert!(joined.contains("twice"), "{joined}");
    }

    #[test]
    fn validate_rejects_cycles() {
        let spec = spec_from(json!({
            "name": "cyc",
            "steps": [
                { "name": "a", "depends_on": ["c"], "kind": { "type": "python", "code": "x" } },
                { "name": "b", "depends_on": ["a"], "kind": { "type": "python", "code": "x" } },
                { "name": "c", "depends_on": ["b"], "kind": { "type": "python", "code": "x" } }
            ]
        }));
        let errs = validate(&spec).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("cycle")), "{errs:?}");
    }

    #[test]
    fn topo_order_is_deterministic_and_layered() {
        let spec = spec_from(json!({
            "name": "dag",
            "steps": [
                { "name": "c", "depends_on": ["b"], "kind": { "type": "python", "code": "x" } },
                { "name": "a", "kind": { "type": "python", "code": "x" } },
                { "name": "b", "depends_on": ["a"], "kind": { "type": "python", "code": "x" } }
            ]
        }));
        assert_eq!(topo_order(&spec).unwrap(), vec!["a", "b", "c"]);
        // Same spec twice -> identical output (spec-order tiebreak).
        assert_eq!(topo_order(&spec).unwrap(), topo_order(&spec).unwrap());
    }

    #[test]
    fn diamond_ready_and_outcome() {
        let spec = spec_from(json!({
            "name": "diamond",
            "steps": [
                { "name": "a", "kind": { "type": "python", "code": "x" } },
                { "name": "b", "depends_on": ["a"], "kind": { "type": "python", "code": "x" } },
                { "name": "c", "depends_on": ["a"], "kind": { "type": "python", "code": "x" } },
                { "name": "d", "depends_on": ["b", "c"], "kind": { "type": "python", "code": "x" } }
            ]
        }));
        assert_eq!(ready_steps(&spec, &states(&[])), vec!["a"]);
        assert_eq!(
            ready_steps(&spec, &states(&[("a", StepOutcome::Done)])),
            vec!["b", "c"]
        );
        assert!(run_outcome(&spec, &states(&[])).is_none());
        assert!(run_outcome(
            &spec,
            &states(&[("a", StepOutcome::Done), ("b", StepOutcome::Done)])
        )
        .is_none());
        // b failed: c still runs, d never becomes ready -> error once c lands.
        let st = states(&[
            ("a", StepOutcome::Done),
            ("b", StepOutcome::Error),
            ("c", StepOutcome::Done),
            ("d", StepOutcome::Cancelled),
        ]);
        assert_eq!(run_outcome(&spec, &st), Some(DagRunStatus::Cancelled));
        let all_done = states(&[
            ("a", StepOutcome::Done),
            ("b", StepOutcome::Done),
            ("c", StepOutcome::Done),
            ("d", StepOutcome::Done),
        ]);
        assert_eq!(run_outcome(&spec, &all_done), Some(DagRunStatus::Done));
    }

    #[test]
    fn context_only_contains_declared_upstream() {
        let spec = spec_from(json!({
            "name": "ctx",
            "steps": [
                { "name": "a", "kind": { "type": "python", "code": "x" } },
                { "name": "b", "kind": { "type": "python", "code": "x" } },
                { "name": "c", "depends_on": ["a"], "kind": { "type": "python", "code": "x" } }
            ]
        }));
        let st = states(&[("a", StepOutcome::Done)]);
        let mut outs = StepOutputs::new();
        outs.insert("a".into(), json!({"rows": 3}));
        let ctx = render_context(&spec, "c", &st, &outs);
        assert_eq!(ctx["steps"]["a"]["json"]["rows"], json!(3));
        assert_eq!(ctx["steps"]["a"]["ok"], json!(true));
        assert!(
            ctx["steps"].get("b").is_none(),
            "sibling b must not leak: {ctx}"
        );
        assert!(ctx["steps"].get("c").is_none());
    }
}
