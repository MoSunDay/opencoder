use super::{ExecutionKind, NodeView, STALE_MS};

/// CPU-normalized projected load. The reservation closes the selection/ack race.
pub fn load(node: &NodeView) -> Option<f64> {
    let s = node.snapshot.as_ref()?;
    if !s.cpu_capacity.is_finite() || s.cpu_capacity <= 0.0 {
        return None;
    }
    Some((s.active_agent_loops.saturating_add(node.reserved_loops)) as f64 / s.cpu_capacity)
}

pub fn eligible(node: &NodeView, kind: ExecutionKind, now: i64) -> bool {
    node.online
        && now.saturating_sub(node.last_seen_at) < STALE_MS
        && node.registration.kinds.contains(&kind)
        && node.snapshot.as_ref().is_some_and(|s| {
            s.ready && s.active_runs.saturating_add(node.reserved_loops) < s.max_runs
        })
        && load(node).is_some()
}

pub fn select_node<'a>(
    nodes: &'a [NodeView],
    kind: ExecutionKind,
    pinned: Option<&str>,
    now: i64,
) -> Option<&'a NodeView> {
    nodes
        .iter()
        .filter(|n| pinned.is_none_or(|id| n.registration.id == id) && eligible(n, kind, now))
        .min_by(|a, b| {
            load(a)
                .unwrap()
                .total_cmp(&load(b).unwrap())
                .then_with(|| {
                    b.snapshot
                        .as_ref()
                        .unwrap()
                        .cpu_capacity
                        .total_cmp(&a.snapshot.as_ref().unwrap().cpu_capacity)
                })
                .then_with(|| a.registration.id.cmp(&b.registration.id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::{NodeRegistration, NodeSnapshot, PROTOCOL_VERSION};
    fn node(id: &str, cpu: f64, loops: u64) -> NodeView {
        NodeView {
            registration: NodeRegistration {
                protocol_version: PROTOCOL_VERSION,
                id: id.into(),
                name: id.into(),
                version: "test".into(),
                maintenance_agent_id: format!("maint-{id}"),
                kinds: vec![ExecutionKind::Agent],
            },
            online: true,
            last_seen_at: 100,
            reserved_loops: 0,
            snapshot: Some(NodeSnapshot {
                generation: "g".into(),
                sequence: 1,
                cpu_capacity: cpu,
                active_agent_loops: loops,
                active_runs: 0,
                max_runs: 10,
                ready: true,
                resource_error: None,
            }),
        }
    }
    #[test]
    fn cpu_weight_reservations_pinning_and_offline() {
        let mut nodes = vec![node("a", 2.0, 2), node("b", 8.0, 4), node("c", 4.0, 3)];
        assert_eq!(
            select_node(&nodes, ExecutionKind::Agent, None, 100)
                .unwrap()
                .registration
                .id,
            "b"
        );
        nodes[1].reserved_loops = 4;
        assert_eq!(
            select_node(&nodes, ExecutionKind::Agent, None, 100)
                .unwrap()
                .registration
                .id,
            "c"
        );
        assert_eq!(
            select_node(&nodes, ExecutionKind::Agent, Some("a"), 100)
                .unwrap()
                .registration
                .id,
            "a"
        );
        nodes[0].online = false;
        assert!(select_node(&nodes, ExecutionKind::Agent, Some("a"), 100).is_none());
        assert!(select_node(&nodes, ExecutionKind::Agent, None, 30_000).is_none());
    }
    #[test]
    fn invalid_cpu_and_duplicate_team_members_are_rejected() {
        assert!(load(&node("n", 0.0, 0)).is_none());
        assert!(load(&node("n", f64::NAN, 0)).is_none());
        assert!(!crate::fleet::valid_id("../x"));
        let team = crate::fleet::TeamDefinition {
            name: "t".into(),
            captain: "a".into(),
            members: vec![],
        };
        assert!(team.validate().is_err());
    }
}
