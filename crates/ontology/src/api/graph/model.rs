use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::domain::{Entity, Relationship};
use serde::Serialize;
use uuid::Uuid;

use super::query::GraphQuery;

#[derive(Serialize)]
pub(super) struct GraphResponse {
    pub nodes: Vec<Entity>,
    pub edges: Vec<Relationship>,
    pub available_relationship_type_ids: Vec<Uuid>,
}

type Neighbors = HashMap<Uuid, Vec<Uuid>>;

fn adjacency(edges: &[Relationship], upstream: bool, bidirectional: bool) -> Neighbors {
    let mut neighbors = Neighbors::new();
    for edge in edges {
        let (source, target) = if upstream {
            (edge.target_entity_id, edge.source_entity_id)
        } else {
            (edge.source_entity_id, edge.target_entity_id)
        };
        neighbors.entry(source).or_default().push(target);
        if bidirectional {
            neighbors.entry(target).or_default().push(source);
        }
    }
    neighbors
}

fn reachable(centers: &HashSet<Uuid>, depth: u8, neighbors: &Neighbors) -> HashSet<Uuid> {
    let mut seen = centers.clone();
    let mut queue = centers.iter().map(|id| (*id, 0)).collect::<VecDeque<_>>();
    while let Some((node, level)) = queue.pop_front() {
        if level >= depth.min(9) {
            continue;
        }
        for neighbor in neighbors.get(&node).into_iter().flatten() {
            if seen.insert(*neighbor) {
                queue.push_back((*neighbor, level + 1));
            }
        }
    }
    seen
}

pub(super) fn observe(
    mut nodes: Vec<Entity>,
    mut edges: Vec<Relationship>,
    query: &GraphQuery,
) -> GraphResponse {
    if query.expand_neighbors {
        return observe_expanded(nodes, edges, query);
    }
    let type_ids = query
        .entity_type_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let mut node_ids = HashSet::new();
    nodes.retain(|node| {
        !node.is_deleted
            && (type_ids.is_empty() || type_ids.contains(&node.entity_type_id))
            && node_ids.insert(node.id)
    });
    let mut edge_ids = HashSet::new();
    edges.retain(|edge| {
        !edge.is_deleted
            && node_ids.contains(&edge.source_entity_id)
            && node_ids.contains(&edge.target_entity_id)
            && (!query.pinned_only || edge.is_pinned)
            && edge_ids.insert(edge.id)
    });
    let centers = query
        .centers
        .iter()
        .copied()
        .filter(|id| node_ids.contains(id))
        .collect::<HashSet<_>>();
    if !query.centers.is_empty() {
        let visible = if query.upstream_depth.is_some() || query.downstream_depth.is_some() {
            let mut ids = reachable(
                &centers,
                query.upstream_depth.unwrap_or(1),
                &adjacency(&edges, true, false),
            );
            ids.extend(reachable(
                &centers,
                query.downstream_depth.unwrap_or(1),
                &adjacency(&edges, false, false),
            ));
            ids
        } else {
            reachable(
                &centers,
                query.depth.unwrap_or(2),
                &adjacency(&edges, false, true),
            )
        };
        nodes.retain(|node| visible.contains(&node.id));
        edges.retain(|edge| {
            visible.contains(&edge.source_entity_id) && visible.contains(&edge.target_entity_id)
        });
    }

    // Candidates describe the complete observation range, independent of the selected types.
    let available_relationship_type_ids = edges
        .iter()
        .map(|edge| edge.relationship_type_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !query.relationship_type_ids.is_empty() {
        let selected = query
            .relationship_type_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        edges.retain(|edge| selected.contains(&edge.relationship_type_id));
    }
    if query.pinned_only || !query.relationship_type_ids.is_empty() {
        let mut visible = centers;
        visible.extend(
            edges
                .iter()
                .flat_map(|edge| [edge.source_entity_id, edge.target_entity_id]),
        );
        nodes.retain(|node| visible.contains(&node.id));
    }
    GraphResponse {
        nodes,
        edges,
        available_relationship_type_ids,
    }
}

fn observe_expanded(
    mut nodes: Vec<Entity>,
    mut edges: Vec<Relationship>,
    query: &GraphQuery,
) -> GraphResponse {
    let mut node_ids = HashSet::new();
    nodes.retain(|node| !node.is_deleted && node_ids.insert(node.id));
    let mut edge_ids = HashSet::new();
    edges.retain(|edge| {
        !edge.is_deleted
            && node_ids.contains(&edge.source_entity_id)
            && node_ids.contains(&edge.target_entity_id)
            && (!query.pinned_only || edge.is_pinned)
            && edge_ids.insert(edge.id)
    });
    let selected_types = query
        .entity_type_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let seeds = if query.centers.is_empty() {
        nodes
            .iter()
            .filter(|node| {
                selected_types.is_empty() || selected_types.contains(&node.entity_type_id)
            })
            .map(|node| node.id)
            .collect::<HashSet<_>>()
    } else {
        query
            .centers
            .iter()
            .copied()
            .filter(|id| {
                nodes.iter().any(|node| {
                    node.id == *id
                        && (selected_types.is_empty()
                            || selected_types.contains(&node.entity_type_id))
                })
            })
            .collect::<HashSet<_>>()
    };
    let up = query.upstream_depth.unwrap_or(1);
    let down = query.downstream_depth.unwrap_or(1);
    let range = |candidate_edges: &[Relationship]| {
        let mut visible = reachable(&seeds, up, &adjacency(candidate_edges, true, false));
        visible.extend(reachable(
            &seeds,
            down,
            &adjacency(candidate_edges, false, false),
        ));
        visible
    };
    let all_visible = range(&edges);
    let available_relationship_type_ids = edges
        .iter()
        .filter(|edge| {
            all_visible.contains(&edge.source_entity_id)
                && all_visible.contains(&edge.target_entity_id)
        })
        .map(|edge| edge.relationship_type_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !query.relationship_type_ids.is_empty() {
        let selected = query
            .relationship_type_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        edges.retain(|edge| selected.contains(&edge.relationship_type_id));
    }
    let visible = range(&edges);
    nodes.retain(|node| visible.contains(&node.id));
    edges.retain(|edge| {
        visible.contains(&edge.source_entity_id) && visible.contains(&edge.target_entity_id)
    });
    GraphResponse {
        nodes,
        edges,
        available_relationship_type_ids,
    }
}
