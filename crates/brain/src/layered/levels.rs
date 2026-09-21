//! Layer derivation. Layers are never stored: save, run and view all call
//! [`layers`], so the canvas cannot drift from the executed schedule.
use anyhow::{ensure, Result};
use opencoder_core::brain::layered::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Kahn layering with a deterministic tie-break on plan order.
pub fn layers(plan: &LayeredPlan) -> Result<Vec<Vec<String>>> {
    validate_shape(plan)?;
    let mut indegree = BTreeMap::new();
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &plan.nodes {
        indegree.insert(node.node_id.as_str(), 0usize);
        next.insert(node.node_id.as_str(), vec![]);
    }
    for edge in &plan.edges {
        *indegree.get_mut(edge.to.as_str()).unwrap() += 1;
        next.get_mut(edge.from.as_str()).unwrap().push(&edge.to);
    }
    let order: Vec<&str> = plan.nodes.iter().map(|n| n.node_id.as_str()).collect();
    let mut done = BTreeSet::new();
    let mut levels = vec![];
    while done.len() < order.len() {
        let ready: Vec<String> = order
            .iter()
            .filter(|id| !done.contains(**id) && indegree[**id] == 0)
            .map(|id| id.to_string())
            .collect();
        ensure!(!ready.is_empty(), "plan edges contain a cycle");
        ensure!(
            ready.len() <= LAYERED_MAX_LAYER_WIDTH,
            "layer width {} exceeds the {LAYERED_MAX_LAYER_WIDTH}-node dispatch limit",
            ready.len()
        );
        for id in &ready {
            done.insert(id.clone());
            for to in next[id.as_str()].iter() {
                *indegree.get_mut(*to).unwrap() -= 1;
            }
        }
        levels.push(ready);
    }
    Ok(levels)
}

pub fn layer_of(plan: &LayeredPlan, node_id: &str) -> Result<u32> {
    for (index, level) in layers(plan)?.iter().enumerate() {
        if level.iter().any(|id| id == node_id) {
            return Ok(index as u32 + 1);
        }
    }
    anyhow::bail!("unknown node {node_id}")
}

/// Every ancestor of every node, computed from the same edge set.
pub fn ancestors(plan: &LayeredPlan) -> Result<BTreeMap<String, BTreeSet<String>>> {
    validate_shape(plan)?;
    let mut incoming: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in &plan.nodes {
        incoming.insert(node.node_id.as_str(), vec![]);
    }
    for edge in &plan.edges {
        incoming.get_mut(edge.to.as_str()).unwrap().push(&edge.from);
    }
    let mut result = BTreeMap::new();
    for node in &plan.nodes {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<&str> = incoming[node.node_id.as_str()].iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            if seen.insert(id.to_string()) {
                queue.extend(incoming[id].iter().copied());
            }
        }
        result.insert(node.node_id.clone(), seen);
    }
    Ok(result)
}

/// Structural shape shared by every entry point: identity, edges and bounds.
pub(crate) fn validate_shape(plan: &LayeredPlan) -> Result<()> {
    ensure!(
        plan.schema_version == LAYERED_SCHEMA_VERSION,
        "unsupported plan schema"
    );
    ensure!(
        !plan.nodes.is_empty() && plan.nodes.len() <= LAYERED_MAX_NODES,
        "a layered plan needs 1..{LAYERED_MAX_NODES} nodes"
    );
    let mut ids = BTreeSet::new();
    for node in &plan.nodes {
        ensure!(
            !node.node_id.trim().is_empty() && node.node_id.len() <= 64,
            "node id must contain 1..64 bytes"
        );
        ensure!(ids.insert(node.node_id.as_str()), "duplicate node id");
        ensure!(
            !node.capability_id.trim().is_empty() && node.capability_id.len() <= 128,
            "each node requires exactly one capability"
        );
        ensure!(
            (1..=5).contains(&node.retry.max_attempts),
            "retry.max_attempts must be 1..5"
        );
        ensure!(
            node.title.chars().count() <= 120,
            "node title must not exceed 120 characters"
        );
    }
    for edge in &plan.edges {
        ensure!(edge.from != edge.to, "self edges are not allowed");
        ensure!(
            ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()),
            "edge references an unknown node"
        );
    }
    let mut unique = BTreeSet::new();
    for edge in &plan.edges {
        ensure!(
            unique.insert((edge.from.as_str(), edge.to.as_str())),
            "duplicate edge"
        );
    }
    Ok(())
}
