//! Validate explicit milestone groups; derive historical schema 4 layers for reads.
use anyhow::{ensure, Result};
use opencoder_core::brain::layered::*;
use std::collections::{BTreeMap, BTreeSet};

/// Kahn layering with a deterministic tie-break on plan order.
fn historical_layers(plan: &LayeredPlan) -> Result<Vec<Vec<String>>> {
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

/// Structural shape shared by every entry point: identity, edges and bounds.
pub(crate) fn validate_shape(plan: &LayeredPlan) -> Result<()> {
    ensure!(
        matches!(plan.schema_version, 4..=7),
        "unsupported plan schema"
    );
    if plan.schema_version >= 6 {
        ensure!(
            plan.edges.is_empty(),
            "schema 6 returns are chosen by Brain; remove configured return edges"
        );
    }
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
            if plan.schema_version == 4 || plan.schema_version == 7 {
                !node.capability_id.trim().is_empty() && node.capability_id.len() <= 128
                    && (plan.schema_version != 7 || node.capability_ids.is_empty())
            } else {
                !node.capability_ids.is_empty()
                    && node.capability_ids.len() <= 32
                    && node
                        .capability_ids
                        .iter()
                        .all(|id| !id.trim().is_empty() && id.len() <= 128)
                    && node.capability_ids.iter().collect::<BTreeSet<_>>().len()
                        == node.capability_ids.len()
                    && node.capability_id.is_empty()
            },
            "schema 7 nodes require one capability; schema 6 nodes require 1..32 unique capabilities"
        );
        if plan.schema_version == 4 {
            ensure!(
                (1..=5).contains(&node.retry.as_ref().map(|r| r.max_attempts).unwrap_or(2)),
                "retry.max_attempts must be 1..5"
            );
        } else {
            ensure!(
                node.retry.is_none(),
                "schema 6 uses Brain reflection rather than per-node retry policies"
            );
        }
        ensure!(
            !node.title.trim().is_empty() && node.title.chars().count() <= 120,
            "node title must contain 1..120 characters"
        );
    }
    for edge in &plan.edges {
        ensure!(
            plan.schema_version >= 5 || edge.from != edge.to,
            "self edges are not allowed"
        );
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
    if plan.schema_version < 7 {
        ensure!(
            plan.layers.is_empty() && plan.transitions.is_empty(),
            "legacy plans cannot contain layer milestones"
        );
    }
    Ok(())
}

/// Explicit parallel groups, independent of cyclic reflection paths.
pub fn layers(plan: &LayeredPlan) -> Result<Vec<Vec<String>>> {
    if plan.schema_version == 4 {
        return historical_layers(plan);
    }
    validate_shape(plan)?;
    if plan.schema_version == 7 {
        ensure!(
            !plan.layers.is_empty() && plan.layers.len() <= 32,
            "plan requires 1..32 milestone layers"
        );
        let mut ids = BTreeSet::new();
        let mut groups = vec![vec![]; plan.layers.len()];
        for (index, layer) in plan.layers.iter().enumerate() {
            ensure!(
                !layer.layer_id.trim().is_empty()
                    && layer.layer_id.len() <= 64
                    && ids.insert(layer.layer_id.as_str()),
                "milestone layer IDs must be unique and 1..64 bytes"
            );
            ensure!(
                !layer.title.trim().is_empty() && layer.title.chars().count() <= 120,
                "milestone title required (max 120)"
            );
            ensure!(
                !layer.objective.trim().is_empty() && layer.objective.chars().count() <= 4096,
                "milestone objective required (max 4096)"
            );
            ensure!(
                !layer.success_criteria.trim().is_empty()
                    && layer.success_criteria.chars().count() <= 4096,
                "milestone success criteria required (max 4096)"
            );
            for node in &plan.nodes {
                if node.layer_id == layer.layer_id {
                    ensure!(
                        node.layer == 0
                            && !node.objective.trim().is_empty()
                            && node.objective.chars().count() <= 4096,
                        "execution node task required (max 4096)"
                    );
                    groups[index].push(node.node_id.clone());
                }
            }
        }
        ensure!(
            plan.nodes
                .iter()
                .all(|node| ids.contains(node.layer_id.as_str())),
            "execution node references unknown layer"
        );
        ensure!(
            groups
                .iter()
                .all(|group| !group.is_empty() && group.len() <= 32),
            "every milestone requires 1..32 execution nodes"
        );
        let mut edges = BTreeSet::new();
        for edge in &plan.transitions {
            let from = plan
                .layers
                .iter()
                .position(|layer| layer.layer_id == edge.from)
                .ok_or_else(|| anyhow::anyhow!("transition source missing"))?;
            let to = plan
                .layers
                .iter()
                .position(|layer| layer.layer_id == edge.to)
                .ok_or_else(|| anyhow::anyhow!("transition target missing"))?;
            ensure!(
                to <= from || to == from + 1,
                "forward transition cannot skip a milestone"
            );
            ensure!(edges.insert((from, to)), "duplicate milestone transition");
            ensure!(
                !edge.condition.trim().is_empty() && edge.condition.chars().count() <= 1024,
                "transition condition required (max 1024)"
            );
        }
        for index in 0..plan.layers.len().saturating_sub(1) {
            ensure!(
                edges.contains(&(index, index + 1)),
                "each nonfinal milestone needs a forward transition"
            );
        }
        return Ok(groups);
    }
    let total = plan.nodes.iter().map(|n| n.layer).max().unwrap_or(0);
    ensure!((1..=32).contains(&total), "plan requires 1..32 layers");
    let mut groups = vec![vec![]; total as usize];
    for node in &plan.nodes {
        ensure!(node.layer > 0, "node layer must be positive");
        ensure!(
            !node.objective.trim().is_empty() && node.objective.chars().count() <= 4096,
            "milestone objective required (max 4096)"
        );
        ensure!(
            !node.success_criteria.trim().is_empty()
                && node.success_criteria.chars().count() <= 4096,
            "milestone success criteria required (max 4096)"
        );
        groups[node.layer as usize - 1].push(node.node_id.clone());
    }
    ensure!(
        groups.iter().all(|g| !g.is_empty() && g.len() <= 32),
        "layers must be contiguous nonempty groups of at most 32 milestones"
    );
    for edge in &plan.edges {
        let from = plan.node(&edge.from).unwrap();
        let to = plan.node(&edge.to).unwrap();
        ensure!(
            to.layer <= from.layer,
            "reflection edges must return to current or previous layers"
        );
        ensure!(
            !edge.condition.trim().is_empty() && edge.condition.chars().count() <= 1024,
            "reflection edge condition required (max 1024)"
        );
    }
    Ok(groups)
}
