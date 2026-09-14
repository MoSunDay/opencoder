use anyhow::{ensure, Result};
use opencoder_core::brain::*;
use std::collections::BTreeSet;

pub(super) fn validate(plan: &OntologyPlan, flow: &ActionFlow) -> Result<()> {
    let ids: BTreeSet<_> = plan.steps.iter().map(|s| s.id.as_str()).collect();
    ensure!(ids.contains(flow.entry.as_str()), "unknown flow entry");
    ensure!(
        (1..=100).contains(&flow.max_visits_per_action),
        "action visit limit must be 1..100"
    );
    ensure!(
        flow.transitions.len() <= 1000,
        "too many action transitions"
    );
    for step in &plan.steps {
        ensure!(
            step.expansion.is_none() && step.when.is_none() && step.depends_on.is_empty(),
            "routed action {} uses transitions instead of foreach/when/depends_on",
            step.id
        );
        let edges: Vec<_> = flow
            .transitions
            .iter()
            .filter(|e| e.from == step.id)
            .collect();
        ensure!(
            !edges.is_empty(),
            "action {} needs a transition or explicit finish",
            step.id
        );
        ensure!(
            edges.iter().filter(|e| e.when.is_none()).count() <= 1,
            "multiple default transitions for {}",
            step.id
        );
        for (index, edge) in edges.iter().enumerate() {
            ensure!(
                !edge.label.trim().is_empty(),
                "transition needs a phenomenon/description"
            );
            ensure!(
                edge.to.as_ref().is_none_or(|to| ids.contains(to.as_str())),
                "unknown transition target"
            );
            if let Some(condition) = &edge.when {
                super::validate::check_condition(plan, step, condition)?;
                ensure!(
                    !edges[..index].iter().any(|other| other.when == edge.when),
                    "duplicate transition condition"
                );
            }
        }
    }
    ensure!(
        flow.transitions
            .iter()
            .all(|e| ids.contains(e.from.as_str())),
        "unknown transition source"
    );
    let mut reachable = BTreeSet::from([flow.entry.as_str()]);
    loop {
        let before = reachable.len();
        for edge in &flow.transitions {
            if reachable.contains(edge.from.as_str()) {
                if let Some(to) = &edge.to {
                    reachable.insert(to.as_str());
                }
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    ensure!(reachable == ids, "flow contains unreachable actions");
    // Every action must have a structural path to an explicit finish.
    let mut finishing: BTreeSet<_> = flow
        .transitions
        .iter()
        .filter(|e| e.to.is_none())
        .map(|e| e.from.as_str())
        .collect();
    loop {
        let before = finishing.len();
        for edge in &flow.transitions {
            if edge
                .to
                .as_ref()
                .is_some_and(|to| finishing.contains(to.as_str()))
            {
                finishing.insert(edge.from.as_str());
            }
        }
        if finishing.len() == before {
            break;
        }
    }
    ensure!(finishing == ids, "every action needs a path to finish");
    Ok(())
}
