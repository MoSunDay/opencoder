use super::{
    model::{observe, GraphResponse},
    query::GraphQuery,
};
use crate::domain::{Entity, Relationship};
use std::collections::HashSet;
use uuid::Uuid;

fn entity(id: u128) -> Entity {
    Entity {
        id: Uuid::from_u128(id),
        env_num: 1,
        entity_type_id: Uuid::from_u128(100),
        name: format!("entity {id}"),
        description: String::new(),
        revision: 1,
        is_deleted: false,
    }
}
fn edge(id: u128, source: u128, target: u128, kind: u128) -> Relationship {
    Relationship {
        id: Uuid::from_u128(id),
        env_num: 1,
        relationship_type_id: Uuid::from_u128(kind),
        source_entity_id: Uuid::from_u128(source),
        target_entity_id: Uuid::from_u128(target),
        description: String::new(),
        revision: 1,
        is_deleted: false,
        is_pinned: false,
    }
}
fn query(centers: &[u128], up: u8, down: u8) -> GraphQuery {
    GraphQuery {
        centers: centers.iter().copied().map(Uuid::from_u128).collect(),
        upstream_depth: Some(up),
        downstream_depth: Some(down),
        ..GraphQuery::default()
    }
}
fn ids(graph: &GraphResponse) -> HashSet<u128> {
    graph.nodes.iter().map(|node| node.id.as_u128()).collect()
}

#[test]
fn repeated_parameters_keep_all_centers_and_types() {
    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let q = GraphQuery::parse(&format!("center={a}&center={b}&center={a}&entity_type_id={a}&entity_type_id={b}&relationship_type_id={a}&relationship_type_id={b}&pinned_only=true")).unwrap();
    assert_eq!(q.centers, vec![a, b]);
    assert_eq!(q.entity_type_ids, vec![a, b]);
    assert_eq!(q.relationship_type_ids, vec![a, b]);
    assert!(q.pinned_only);
    assert!(
        GraphQuery::parse("expand_neighbors=true")
            .unwrap()
            .expand_neighbors
    );
    for raw in [
        "center=bad",
        "entity_type_id=bad",
        "upstream_depth=-1",
        "downstream_depth=bad",
        "pinned_only=yes",
        "expand_neighbors=yes",
        "aspect_id=old",
        "unknown=value",
    ] {
        assert!(GraphQuery::parse(raw).is_err(), "{raw}");
    }
}

#[test]
fn expanded_observation_uses_selected_types_as_seeds_and_crosses_type_boundaries() {
    let mut other = entity(2);
    other.entity_type_id = Uuid::from_u128(200);
    let mut third = entity(3);
    third.entity_type_id = Uuid::from_u128(300);
    let mut q = query(&[1], 0, 2);
    q.entity_type_ids = vec![Uuid::from_u128(100)];
    q.expand_neighbors = true;
    let nodes = vec![entity(1), other, third];
    let edges = vec![edge(11, 1, 2, 101), edge(12, 2, 3, 102)];
    let all = observe(nodes.clone(), edges.clone(), &q);
    assert_eq!(ids(&all), HashSet::from([1, 2, 3]));
    assert_eq!(all.edges.len(), 2);
    q.relationship_type_ids = vec![Uuid::from_u128(102)];
    let filtered = observe(nodes, edges, &q);
    assert_eq!(ids(&filtered), HashSet::from([1]));
    assert!(filtered.edges.is_empty());
    assert_eq!(filtered.available_relationship_type_ids.len(), 2);
}

#[test]
fn expanded_observation_without_center_uses_all_entities_of_seed_type() {
    let mut other = entity(3);
    other.entity_type_id = Uuid::from_u128(200);
    let mut q = query(&[], 0, 1);
    q.entity_type_ids = vec![Uuid::from_u128(100)];
    q.expand_neighbors = true;
    let graph = observe(
        vec![entity(1), entity(2), other],
        vec![edge(11, 2, 3, 101)],
        &q,
    );
    assert_eq!(ids(&graph), HashSet::from([1, 2, 3]));
    assert_eq!(graph.edges.len(), 1);
}

#[test]
fn directional_depths_are_independent_and_multicenter_ranges_are_unioned() {
    let nodes = (1..=7).map(entity).collect();
    let edges = vec![
        edge(11, 1, 2, 101),
        edge(12, 2, 3, 101),
        edge(13, 3, 4, 102),
        edge(14, 4, 5, 102),
        edge(15, 6, 7, 103),
    ];
    let graph = observe(nodes, edges, &query(&[3, 6, 3], 2, 1));
    assert_eq!(ids(&graph), HashSet::from([1, 2, 3, 4, 6, 7]));
    assert_eq!(graph.edges.len(), 4);
    assert_eq!(graph.available_relationship_type_ids.len(), 3);
}

#[test]
fn relationship_filter_applies_after_range_and_preserves_centers() {
    let nodes = (1..=4).map(entity).collect::<Vec<_>>();
    let edges = vec![
        edge(11, 1, 2, 101),
        edge(12, 2, 3, 102),
        edge(13, 3, 4, 103),
    ];
    let mut q = query(&[1], 0, 2);
    q.relationship_type_ids = vec![Uuid::from_u128(102)];
    let graph = observe(nodes.clone(), edges.clone(), &q);
    assert_eq!(ids(&graph), HashSet::from([1, 2, 3]));
    assert_eq!(
        graph
            .edges
            .iter()
            .map(|edge| edge.id.as_u128())
            .collect::<Vec<_>>(),
        vec![12]
    );
    assert_eq!(
        graph.available_relationship_type_ids,
        vec![Uuid::from_u128(101), Uuid::from_u128(102)]
    );
    q.relationship_type_ids.clear();
    let all = observe(nodes, edges, &q);
    assert_eq!(all.edges.len(), 2);
    assert_eq!(
        all.available_relationship_type_ids,
        graph.available_relationship_type_ids
    );
}

#[test]
fn zero_hops_cycles_and_isolated_centers_are_bounded() {
    let nodes = (1..=3).map(entity).collect::<Vec<_>>();
    let edges = vec![edge(11, 1, 2, 101), edge(12, 2, 1, 101)];
    assert_eq!(
        ids(&observe(nodes.clone(), edges.clone(), &query(&[1], 0, 0))),
        HashSet::from([1])
    );
    let all = observe(nodes.clone(), edges.clone(), &query(&[1, 3], 5, 5));
    assert_eq!(ids(&all), HashSet::from([1, 2, 3]));
    assert_eq!(all.edges.len(), 2);
    let zero = observe(nodes, edges, &query(&[1, 2], 0, 0));
    assert_eq!(zero.edges.len(), 2);
}

#[test]
fn absent_filter_type_keeps_only_centers_but_candidates_remain_available() {
    let mut q = query(&[1, 4], 0, 2);
    q.relationship_type_ids = vec![Uuid::from_u128(999)];
    let graph = observe(
        (1..=4).map(entity).collect(),
        vec![edge(11, 1, 2, 101), edge(12, 2, 3, 102)],
        &q,
    );
    assert_eq!(ids(&graph), HashSet::from([1, 4]));
    assert!(graph.edges.is_empty());
    assert_eq!(graph.available_relationship_type_ids.len(), 2);
}

#[test]
fn deleted_nodes_edges_and_dangling_endpoints_never_enter_candidates() {
    let mut deleted = entity(3);
    deleted.is_deleted = true;
    let mut deleted_edge = edge(12, 1, 2, 102);
    deleted_edge.is_deleted = true;
    let graph = observe(
        vec![entity(1), entity(1), entity(2), deleted],
        vec![
            edge(11, 1, 2, 101),
            edge(11, 1, 2, 101),
            deleted_edge,
            edge(13, 2, 3, 103),
            edge(14, 2, 99, 104),
        ],
        &query(&[1, 3], 2, 2),
    );
    assert_eq!(ids(&graph), HashSet::from([1, 2]));
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(
        graph.available_relationship_type_ids,
        vec![Uuid::from_u128(101)]
    );
}

#[test]
fn fixed_graph_contains_only_pinned_edges_and_endpoints() {
    let mut pinned = edge(11, 1, 2, 101);
    pinned.is_pinned = true;
    let graph = observe(
        (1..=4).map(entity).collect(),
        vec![pinned, edge(12, 2, 3, 102)],
        &GraphQuery {
            pinned_only: true,
            ..GraphQuery::default()
        },
    );
    assert_eq!(ids(&graph), HashSet::from([1, 2]));
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(
        graph.available_relationship_type_ids,
        vec![Uuid::from_u128(101)]
    );
}

#[test]
fn legacy_single_center_depth_and_entity_type_filter_still_work() {
    let a = Uuid::from_u128(1);
    let mut nodes = (1..=4).map(entity).collect::<Vec<_>>();
    nodes[3].entity_type_id = Uuid::from_u128(200);
    let edges = vec![
        edge(11, 1, 2, 101),
        edge(12, 3, 2, 102),
        edge(13, 3, 4, 103),
    ];
    let q = GraphQuery::parse(&format!("center={a}&depth=2")).unwrap();
    assert_eq!(
        ids(&observe(nodes.clone(), edges.clone(), &q)),
        HashSet::from([1, 2, 3])
    );
    let graph = observe(
        nodes,
        edges,
        &GraphQuery {
            entity_type_ids: vec![Uuid::from_u128(100)],
            ..GraphQuery::default()
        },
    );
    assert_eq!(ids(&graph), HashSet::from([1, 2, 3]));
    assert_eq!(graph.edges.len(), 2);
}

#[test]
fn missing_centers_do_not_expand_to_the_entire_env() {
    let graph = observe(
        vec![entity(1), entity(2)],
        vec![edge(11, 1, 2, 101)],
        &query(&[99], 1, 1),
    );
    assert!(graph.nodes.is_empty());
    assert!(graph.edges.is_empty());
    assert!(graph.available_relationship_type_ids.is_empty());
}

#[test]
fn graph_observation_does_not_truncate_large_inputs() {
    let nodes = (1..=10_002).map(entity).collect();
    let edges = (1..=10_001)
        .map(|id| edge(id + 20_000, id, id + 1, 101))
        .collect();
    let graph = observe(nodes, edges, &query(&[10_002], 1, 0));
    assert_eq!(ids(&graph), HashSet::from([10_001, 10_002]));
    assert_eq!(graph.edges.len(), 1);
}

#[test]
fn depth_clamps_at_nine_hops_for_aspect_observation() {
    // 链式图 1->2->...->11，切面观测需要 9 跳，超过 9 的请求与 9 结果相同。
    let nodes = (1..=11).map(entity).collect::<Vec<_>>();
    let edges = (1..=10)
        .map(|id| edge(id + 20_000, id, id + 1, 101))
        .collect::<Vec<_>>();
    let downstream = observe(nodes.clone(), edges.clone(), &query(&[1], 0, 9));
    assert_eq!(
        ids(&downstream),
        (1..=10).collect::<HashSet<u128>>(),
        "downstream depth 9 reaches the 9th neighbor"
    );
    assert_eq!(
        ids(&observe(nodes.clone(), edges.clone(), &query(&[1], 0, 15))),
        ids(&downstream)
    );

    let upstream = observe(nodes.clone(), edges.clone(), &query(&[11], 9, 0));
    assert_eq!(
        ids(&upstream),
        (2..=11).collect::<HashSet<u128>>(),
        "upstream depth 9 reaches the 9th neighbor"
    );
    assert_eq!(
        ids(&observe(nodes, edges, &query(&[11], 15, 0))),
        ids(&upstream)
    );
}
