//! Historical decision-tree decoding and structural validation.
//! Execution is retired; new plans use the v2 graph kernel.

use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Hard ceiling on tree depth (the planner prompt asks for ≤ 4; this is the
/// validator's backstop so a rambling model can never build a degenerate
/// spine).
pub const MAX_TREE_DEPTH: usize = 6;
/// Hard ceiling on leaf count (prompt asks for ≤ 8; backstop only).
pub const MAX_LEAVES: usize = 16;
/// Longest accepted branch topic, in chars (prompt asks for ≤ 16; topics are
/// embedded verbatim, so long essays would blur the routing signal).
pub const MAX_TOPIC_CHARS: usize = 64;

/// A validated routing tree. `threshold` is the cosine-similarity cut every
/// branch applies (set by the planner, defaulting near 0.35 for real
/// semantic embeddings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTree {
    pub threshold: f64,
    pub root: PlanNode,
}

/// One node of a [`DecisionTree`]. Tagged serde (`kind`: `branch` | `leaf`)
/// is the exact wire shape the planner prompt mandates, so the LLM's raw
/// JSON parses straight into this enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PlanNode {
    Branch {
        id: String,
        /// Short discriminative topic phrase; embedded at plan time and
        /// attached as `topic_vec` before the tree is persisted.
        topic: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        topic_vec: Option<Vec<f32>>,
        yes: Box<PlanNode>,
        no: Box<PlanNode>,
    },
    Leaf {
        id: String,
        capability_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// One hop of a dispatch walk — the audit trail of why a situation landed
/// on a capability.
#[derive(Debug, Clone, Serialize)]
pub struct DispatchStep {
    pub node_id: String,
    /// `"branch"` or `"leaf"`.
    pub kind: &'static str,
    pub topic: Option<String>,
    /// Cosine similarity at a branch (`situation` vs `topic_vec`).
    pub score: Option<f64>,
    /// Which child a branch took (`true` = yes).
    pub taken: Option<bool>,
}

/// The result of routing one situation through a [`DecisionTree`].
#[derive(Debug, Clone, Serialize)]
pub struct DispatchOutcome {
    pub capability_id: String,
    pub reason: Option<String>,
    pub path: Vec<DispatchStep>,
}

/// Validate the structural contract: threshold sane, ids unique and
/// non-empty, topics short and non-empty, every leaf's `capability_id` drawn
/// from the retrieved candidate set, depth/leaf budgets respected, at least
/// one leaf. Rejects as plain anyhow (caller maps to the typed
/// plan-generation marker).
pub fn validate(tree: &DecisionTree, candidate_ids: &HashSet<&str>) -> Result<()> {
    if !tree.threshold.is_finite() || tree.threshold <= 0.0 || tree.threshold > 1.0 {
        bail!("tree threshold must be in (0, 1], got {}", tree.threshold);
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut leaves = 0usize;
    walk_validate(&tree.root, 1, candidate_ids, &mut seen, &mut leaves)?;
    if leaves == 0 {
        bail!("tree has no leaf");
    }
    Ok(())
}

fn walk_validate<'a>(
    node: &'a PlanNode,
    depth: usize,
    candidate_ids: &HashSet<&str>,
    seen: &mut HashSet<&'a str>,
    leaves: &mut usize,
) -> Result<()> {
    if depth > MAX_TREE_DEPTH {
        bail!("tree depth exceeds {MAX_TREE_DEPTH}");
    }
    match node {
        PlanNode::Branch {
            id, topic, yes, no, ..
        } => {
            if id.is_empty() {
                bail!("branch node with empty id");
            }
            if !seen.insert(id.as_str()) {
                bail!("duplicate node id: {id}");
            }
            let chars = topic.trim().chars().count();
            if chars == 0 {
                bail!("branch {id} has an empty topic");
            }
            if chars > MAX_TOPIC_CHARS {
                bail!("branch {id} topic exceeds {MAX_TOPIC_CHARS} chars");
            }
            walk_validate(yes, depth + 1, candidate_ids, seen, leaves)?;
            walk_validate(no, depth + 1, candidate_ids, seen, leaves)
        }
        PlanNode::Leaf {
            id, capability_id, ..
        } => {
            if id.is_empty() {
                bail!("leaf node with empty id");
            }
            if !seen.insert(id.as_str()) {
                bail!("duplicate node id: {id}");
            }
            if !candidate_ids.contains(capability_id.as_str()) {
                bail!("leaf {id} references unknown capability_id: {capability_id}");
            }
            *leaves += 1;
            if *leaves > MAX_LEAVES {
                bail!("tree exceeds {MAX_LEAVES} leaves");
            }
            Ok(())
        }
    }
}

/// Collect every branch topic in deterministic pre-order — the exact order
/// [`attach_topic_vectors`] consumes embeddings in.
pub fn collect_topics(node: &PlanNode, out: &mut Vec<String>) {
    match node {
        PlanNode::Branch { topic, yes, no, .. } => {
            out.push(topic.clone());
            collect_topics(yes, out);
            collect_topics(no, out);
        }
        PlanNode::Leaf { .. } => {}
    }
}

/// Attach plan-time topic embeddings to every branch (in `collect_topics`
/// order), L2-normalizing each. Rejects cardinality mismatches, empty or
/// zero vectors, and ragged dimensions — a tree that passes is fully
/// dispatchable.
pub fn attach_topic_vectors(root: &mut PlanNode, vecs: &[Vec<f32>]) -> Result<()> {
    let mut topics = Vec::new();
    collect_topics(root, &mut topics);
    if vecs.len() != topics.len() {
        bail!(
            "expected {} topic vectors, got {}",
            topics.len(),
            vecs.len()
        );
    }
    let dim = vecs.first().map(|v| v.len()).unwrap_or(0);
    for (i, v) in vecs.iter().enumerate() {
        if v.is_empty() {
            bail!("topic vector {i} is empty");
        }
        if v.len() != dim {
            bail!("topic vector {i} has dim {}, expected {dim}", v.len());
        }
    }
    let mut idx = 0usize;
    attach_walk(root, vecs, &mut idx);
    Ok(())
}

fn attach_walk(node: &mut PlanNode, vecs: &[Vec<f32>], idx: &mut usize) {
    match node {
        PlanNode::Branch {
            topic_vec, yes, no, ..
        } => {
            let mut v = vecs[*idx].clone();
            normalize(&mut v);
            *topic_vec = Some(v);
            *idx += 1;
            attach_walk(yes, vecs, idx);
            attach_walk(no, vecs, idx);
        }
        PlanNode::Leaf { .. } => {}
    }
}

/// Retired decision-tree execution entry point; historical decoding remains.
pub fn dispatch(_tree: &DecisionTree, _situation: &[f32]) -> Result<DispatchOutcome> {
    bail!(crate::graph::MIGRATION)
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in v {
            *x = (*x as f64 / norm) as f32;
        }
    }
}
