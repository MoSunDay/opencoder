//! Pure topology helpers over a [`PlaybookSpec`]: deterministic topological
//! order, the ready set, failure collapse, and the chain-depth / fan-out
//! width metrics [`super::spec::validate`] enforces. No I/O, no clocks —
//! every function is a pure fold over the spec's `depends_on` edges.

use std::collections::{BTreeMap, BTreeSet};

use super::spec::PlaybookSpec;

/// Deterministic Kahn topological order: the ready set is always ordered
/// lexicographically by step name (a `BTreeSet`), so the same spec yields
/// the same schedule everywhere — executor batching, UI listings and tests
/// all agree. A dependency naming a step that does not exist is an error
/// (callers that pre-validated never hit it); a cycle is reported naming a
/// representative stuck step.
pub fn topo_order(spec: &PlaybookSpec) -> Result<Vec<String>, String> {
    let names: BTreeSet<&str> = spec.steps.iter().map(|s| s.name.as_str()).collect();
    for step in &spec.steps {
        for dep in &step.depends_on {
            if !names.contains(dep.as_str()) {
                return Err(format!(
                    "step {:?} depends on unknown step {:?}",
                    step.name, dep
                ));
            }
        }
    }
    // Deduplicated dependency edges (validation rejects duplicates; the
    // dedup keeps the indegree arithmetic sound on raw input).
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for step in &spec.steps {
        let deps: BTreeSet<&str> = step.depends_on.iter().map(|d| d.as_str()).collect();
        for dep in &deps {
            dependents.entry(dep).or_default().push(step.name.as_str());
        }
        indegree.insert(step.name.as_str(), deps.len());
    }
    let mut ready: BTreeSet<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut order: Vec<String> = Vec::with_capacity(spec.steps.len());
    while let Some(name) = ready.iter().next().copied() {
        ready.remove(&name);
        order.push(name.to_string());
        for dep in dependents.get(name).into_iter().flatten() {
            if let Some(slot) = indegree.get_mut(dep) {
                *slot = slot.saturating_sub(1);
                if *slot == 0 {
                    ready.insert(dep);
                }
            }
        }
    }
    if order.len() != spec.steps.len() {
        let stuck: Vec<&str> = indegree
            .iter()
            .filter(|(_, d)| **d > 0)
            .map(|(n, _)| *n)
            .collect();
        let rep = stuck.iter().min().copied().unwrap_or("");
        return Err(format!("cycle detected involving step {rep:?}"));
    }
    Ok(order)
}

/// Steps not in `done` whose every dependency is in `done`, sorted
/// lexicographically — the executor's entire next-batch decision.
pub fn ready_steps(spec: &PlaybookSpec, done: &BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = spec
        .steps
        .iter()
        .filter(|s| !done.contains(&s.name))
        .filter(|s| s.depends_on.iter().all(|d| done.contains(d)))
        .map(|s| s.name.clone())
        .collect();
    out.sort();
    out
}

/// The transitive downstream of `failed`: every step whose dependency
/// closure intersects `failed`, plus the failed steps themselves. This is
/// the collapse set the executor skips (they can never become ready).
pub fn collapse_blocked(spec: &PlaybookSpec, failed: &BTreeSet<String>) -> BTreeSet<String> {
    let mut blocked: BTreeSet<String> = failed.clone();
    loop {
        let mut grew = false;
        for step in &spec.steps {
            if blocked.contains(&step.name) {
                continue;
            }
            if step.depends_on.iter().any(|d| blocked.contains(d)) {
                grew |= blocked.insert(step.name.clone());
            }
        }
        if !grew {
            return blocked;
        }
    }
}

/// Longest dependency chain (Kahn levels minus one). Only meaningful for
/// acyclic specs: an empty spec reports 0, and a cyclic one reports
/// `steps.len() + 1` — a value past every limit so [`super::spec::validate`]
/// (which already pushed the topology error) flags it either way.
pub fn chain_depth(spec: &PlaybookSpec) -> usize {
    match kahn_levels(spec) {
        Some(levels) => levels.len().saturating_sub(1),
        None => spec.steps.len() + 1,
    }
}

/// Widest ready batch over a simulated schedule (the largest Kahn level).
/// Only meaningful for acyclic specs; cycles report `steps.len() + 1`, past
/// every limit (see [`chain_depth`]).
pub fn max_width(spec: &PlaybookSpec) -> usize {
    match kahn_levels(spec) {
        Some(levels) => levels.iter().map(|l| l.len()).max().unwrap_or(0),
        None => spec.steps.len() + 1,
    }
}

/// Simulated Kahn schedule as levels: level 0 = the initial roots, level n
/// = the steps becoming ready once levels 0..n are done. `None` when the
/// spec is cyclic (or a dependency names an unknown step — nothing ever
/// becomes ready, the same stall).
fn kahn_levels(spec: &PlaybookSpec) -> Option<Vec<Vec<String>>> {
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut levels: Vec<Vec<String>> = Vec::new();
    while done.len() < spec.steps.len() {
        let batch = ready_steps(spec, &done);
        if batch.is_empty() {
            return None;
        }
        for name in &batch {
            done.insert(name.clone());
        }
        levels.push(batch);
    }
    Some(levels)
}
