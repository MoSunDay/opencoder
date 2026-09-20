use opencoder_core::{fleet::*, message::now_ms};
use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::SocketCommand;

/// Most node RPCs are local reads or control commands and should fail fast
/// when a node stops responding.  Creation is different: the node snapshots
/// the configured agent resources before it can acknowledge the assignment,
/// and a first snapshot may legitimately take tens of seconds on NFS.  Keep
/// that slow admission window separate so a large resource pool does not turn
/// a successful operator/agent submission into a spurious 504.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CREATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn request_timeout(operation: &NodeOperation) -> Duration {
    match operation {
        NodeOperation::Create { .. } => CREATE_REQUEST_TIMEOUT,
        _ => DEFAULT_REQUEST_TIMEOUT,
    }
}

struct Connection {
    generation: String,
    tx: mpsc::Sender<SocketCommand>,
    reported_ready: bool,
    index_synced: bool,
}
struct Standby {
    registration: NodeRegistration,
    snapshot: NodeSnapshot,
    connection: Connection,
}

struct Pending {
    node_id: String,
    generation: String,
    tx: oneshot::Sender<RpcReply>,
    execution: Option<ExecutionIndex>,
}
struct Reservation {
    node_id: String,
    claims: u64,
}
pub(super) struct CreateReply {
    pub execution: ExecutionIndex,
    pub reply: RpcReply,
    pub late: bool,
    pub reject_pending: bool,
}
struct State {
    nodes: HashMap<String, NodeView>,
    connections: HashMap<String, Connection>,
    standby: HashMap<String, Standby>,
    pending: HashMap<String, Pending>,
    reservations: HashMap<String, Reservation>,
    accepted: HashMap<String, String>,
}
pub struct Hub {
    state: Mutex<State>,
}

pub enum UnregisterResult {
    Removed,
    NotFound,
    Connected,
}

impl Hub {
    pub fn new(nodes: Vec<NodeRegistration>) -> Self {
        let nodes = nodes
            .into_iter()
            .map(|registration| {
                (
                    registration.id.clone(),
                    NodeView {
                        registration,
                        online: false,
                        last_seen_at: 0,
                        snapshot: None,
                        reserved_loops: 0,
                    },
                )
            })
            .collect();
        Self {
            state: Mutex::new(State {
                nodes,
                connections: HashMap::new(),
                standby: HashMap::new(),
                pending: HashMap::new(),
                reservations: HashMap::new(),
                accepted: HashMap::new(),
            }),
        }
    }
    pub async fn views(&self) -> Vec<NodeView> {
        let mut nodes: Vec<_> = self.state.lock().await.nodes.values().cloned().collect();
        for node in &mut nodes {
            node.online &= now_ms().saturating_sub(node.last_seen_at) < STALE_MS;
        }
        nodes.sort_by(|a, b| a.registration.id.cmp(&b.registration.id));
        nodes
    }
    /// Serialize removal with attach and keep the live view intact if the
    /// durable delete fails. Active connections must be stopped first: the
    /// worker otherwise re-registers automatically and continues local work.
    pub async fn unregister(
        &self,
        id: &str,
        fleet: &opencoder_store::fleet::FleetStore,
    ) -> anyhow::Result<UnregisterResult> {
        let mut state = self.state.lock().await;
        if state.connections.contains_key(id) {
            return Ok(UnregisterResult::Connected);
        }
        let removed = fleet.unregister(id).await?;
        let cached = state.nodes.remove(id).is_some();
        Ok(if removed || cached {
            UnregisterResult::Removed
        } else {
            UnregisterResult::NotFound
        })
    }
    pub async fn close_connections(&self) {
        let senders: Vec<_> = self
            .state
            .lock()
            .await
            .connections
            .values()
            .map(|connection| connection.tx.clone())
            .collect();
        futures::future::join_all(senders.into_iter().map(|tx| async move {
            let _ =
                tokio::time::timeout(Duration::from_secs(5), tx.send(SocketCommand::Close)).await;
        }))
        .await;
    }
    pub(super) async fn attach(
        &self,
        registration: NodeRegistration,
        mut snapshot: NodeSnapshot,
        tx: mpsc::Sender<SocketCommand>,
    ) -> anyhow::Result<()> {
        if registration.protocol_version != PROTOCOL_VERSION {
            anyhow::bail!(
                "incompatible node protocol {}; server requires {}: upgrade server and node together",
                registration.protocol_version,
                PROTOCOL_VERSION
            );
        }
        if !valid_id(&registration.id)
            || registration.name.trim().is_empty()
            || snapshot.generation.trim().is_empty()
            || !snapshot.cpu_capacity.is_finite()
            || snapshot.cpu_capacity <= 0.0
            || snapshot.max_runs == 0
        {
            anyhow::bail!("invalid node registration");
        }
        let mut state = self.state.lock().await;
        if let Some(current) = state.connections.get(&registration.id) {
            let epoch = |s: &str| {
                s.strip_prefix("host-")
                    .and_then(|s| s.split('-').next())
                    .and_then(|s| s.parse::<u64>().ok())
            };
            anyhow::ensure!(
                matches!((epoch(&snapshot.generation),epoch(&current.generation)), (Some(new),Some(old)) if new > old),
                "node already has an active connection"
            );
            if let Some(pending) = state.standby.get(&registration.id) {
                anyhow::ensure!(
                    epoch(&snapshot.generation) > epoch(&pending.snapshot.generation),
                    "stale standby host"
                );
            }
            state.standby.insert(
                registration.id.clone(),
                Standby {
                    connection: Connection {
                        generation: snapshot.generation.clone(),
                        tx,
                        reported_ready: snapshot.ready,
                        index_synced: false,
                    },
                    registration,
                    snapshot,
                },
            );
            return Ok(());
        }
        let reported_ready = snapshot.ready;
        snapshot.ready = false;
        state.connections.insert(
            registration.id.clone(),
            Connection {
                generation: snapshot.generation.clone(),
                tx,
                reported_ready,
                index_synced: false,
            },
        );
        let reserved_loops = state
            .reservations
            .values()
            .filter(|reservation| reservation.node_id == registration.id)
            .map(|reservation| reservation.claims)
            .sum();
        state.nodes.insert(
            registration.id.clone(),
            NodeView {
                registration,
                online: true,
                last_seen_at: now_ms(),
                snapshot: Some(snapshot),
                reserved_loops,
            },
        );
        Ok(())
    }
    pub async fn snapshot(&self, id: &str, generation: &str, mut snapshot: NodeSnapshot) {
        let mut state = self.state.lock().await;
        if let Some(standby) = state
            .standby
            .get_mut(id)
            .filter(|s| s.connection.generation == generation)
        {
            if snapshot.generation == generation && snapshot.sequence > standby.snapshot.sequence {
                standby.connection.reported_ready = snapshot.ready;
                standby.snapshot = snapshot;
            }
            return;
        }
        let (index_synced, reported_ready) = match state.connections.get_mut(id) {
            Some(connection)
                if connection.generation == generation && snapshot.generation == generation =>
            {
                connection.reported_ready = snapshot.ready;
                (connection.index_synced, connection.reported_ready)
            }
            _ => return,
        };
        if let Some(node) = state.nodes.get_mut(id) {
            if node.snapshot.as_ref().is_some_and(|old| {
                old.generation != snapshot.generation || old.sequence >= snapshot.sequence
            }) || !snapshot.cpu_capacity.is_finite()
                || snapshot.cpu_capacity <= 0.0
            {
                return;
            }
            snapshot.ready = index_synced && reported_ready;
            node.snapshot = Some(snapshot);
            node.last_seen_at = now_ms();
        }
    }
    pub async fn touch(&self, id: &str, generation: &str) -> bool {
        let mut state = self.state.lock().await;
        if state
            .standby
            .get(id)
            .is_some_and(|s| s.connection.generation == generation)
        {
            return true;
        }
        if state
            .connections
            .get(id)
            .is_none_or(|connection| connection.generation != generation)
        {
            return false;
        }
        if let Some(node) = state.nodes.get_mut(id) {
            node.last_seen_at = now_ms();
        }
        true
    }
    pub async fn mark_index_synced(&self, id: &str, generation: &str) -> bool {
        let mut state = self.state.lock().await;
        if state
            .standby
            .get(id)
            .is_some_and(|s| s.connection.generation == generation)
        {
            let mut standby = state.standby.remove(id).unwrap();
            standby.connection.index_synced = true;
            standby.snapshot.ready = standby.connection.reported_ready;
            state.connections.insert(id.to_string(), standby.connection);
            if let Some(node) = state.nodes.get_mut(id) {
                node.registration = standby.registration;
                node.snapshot = Some(standby.snapshot);
                node.online = true;
                node.last_seen_at = now_ms();
            }
            return true;
        }
        let ready = match state.connections.get_mut(id) {
            Some(connection) if connection.generation == generation => {
                connection.index_synced = true;
                connection.reported_ready
            }
            _ => return false,
        };
        if let Some(node) = state.nodes.get_mut(id) {
            if let Some(snapshot) = node.snapshot.as_mut() {
                snapshot.ready = ready;
            }
            node.last_seen_at = now_ms();
        }
        true
    }
    pub async fn reserve(&self, record: &ExecutionIndex) {
        let mut state = self.state.lock().await;
        let reservation = state
            .reservations
            .entry(record.id.clone())
            .or_insert_with(|| Reservation {
                node_id: record.node_id.clone(),
                claims: 0,
            });
        if reservation.node_id != record.node_id {
            return;
        }
        reservation.claims = reservation.claims.saturating_add(1);
        if let Some(node) = state.nodes.get_mut(&record.node_id) {
            node.reserved_loops = node.reserved_loops.saturating_add(1);
        }
    }
    pub async fn clear_reservations(&self, records: &[ExecutionIndex]) {
        let mut state = self.state.lock().await;
        for record in records {
            let Some(reservation) = state.reservations.remove(&record.id) else {
                continue;
            };
            if let Some(node) = state.nodes.get_mut(&reservation.node_id) {
                node.reserved_loops = node.reserved_loops.saturating_sub(reservation.claims);
            }
        }
    }
    pub async fn acknowledge_report(&self, node_id: &str, records: &[ExecutionIndex]) {
        let mut state = self.state.lock().await;
        // Replace acceptance evidence only after a complete report.
        state.accepted.retain(|_, node| node != node_id);
        for record in records {
            state
                .accepted
                .insert(record.id.clone(), record.node_id.clone());
        }
    }
    pub async fn detach(&self, id: &str, generation: &str) {
        let mut state = self.state.lock().await;
        if state
            .standby
            .get(id)
            .is_some_and(|s| s.connection.generation == generation)
        {
            state.standby.remove(id);
        }
        if state
            .connections
            .get(id)
            .is_some_and(|c| c.generation == generation)
        {
            state.connections.remove(id);
            if let Some(node) = state.nodes.get_mut(id) {
                node.online = false;
            }
        }
        // A promoted Host does not own the older socket's pending replies.
        // Settle that socket's calls even when the current connection differs.
        let requests: Vec<_> = state
            .pending
            .iter()
            .filter(|(_, p)| p.node_id == id && p.generation == generation)
            .map(|(id, _)| id.clone())
            .collect();
        for request in requests {
            if let Some(p) = state.pending.remove(&request) {
                let _ = p.tx.send(RpcReply::error(
                    503,
                    "node disconnected; assignment retained",
                ));
            }
        }
    }
    pub(super) async fn resolve(
        &self,
        node: &str,
        generation: &str,
        id: &str,
        reply: RpcReply,
    ) -> Option<CreateReply> {
        let mut state = self.state.lock().await;
        if state
            .pending
            .get(id)
            .is_some_and(|p| p.node_id == node && p.generation == generation)
        {
            if let Some(pending) = state.pending.remove(id) {
                let Some(execution) = pending.execution else {
                    let _ = pending.tx.send(reply);
                    return None;
                };
                let accepted = (200..300).contains(&reply.status);
                if accepted {
                    state
                        .accepted
                        .insert(execution.id.clone(), execution.node_id.clone());
                }
                let remaining_claims = release_claim(&mut state, &execution);
                let reject_pending = !accepted
                    && remaining_claims == Some(0)
                    && !state.accepted.contains_key(&execution.id);
                let late = pending.tx.send(reply.clone()).is_err();
                return Some(CreateReply {
                    execution,
                    reply,
                    late,
                    reject_pending,
                });
            }
        }
        None
    }
    pub async fn call(&self, node_id: &str, operation: NodeOperation) -> RpcReply {
        let timeout = request_timeout(&operation);
        self.call_with_timeout(node_id, operation, timeout).await
    }
    async fn call_with_timeout(
        &self,
        node_id: &str,
        operation: NodeOperation,
        timeout: Duration,
    ) -> RpcReply {
        let id = ulid::Ulid::new().to_string();
        let (tx, rx) = oneshot::channel();
        let execution = match &operation {
            NodeOperation::Create { assignment } => Some(assignment.index.clone()),
            _ => None,
        };
        {
            let mut state = self.state.lock().await;
            if state.pending.len() >= 1024 {
                return reject_undispatched(&mut state, execution.as_ref(), "control channel busy");
            }
            let Some(connection) = state.connections.get(node_id) else {
                return reject_undispatched(&mut state, execution.as_ref(), "node offline");
            };
            if !connection.index_synced {
                return reject_undispatched(
                    &mut state,
                    execution.as_ref(),
                    "node initial index sync pending",
                );
            }
            if state
                .nodes
                .get(node_id)
                .is_none_or(|n| now_ms().saturating_sub(n.last_seen_at) >= STALE_MS)
            {
                return reject_undispatched(&mut state, execution.as_ref(), "node heartbeat stale");
            }
            let generation = connection.generation.clone();
            if connection
                .tx
                .try_send(SocketCommand::Frame(Box::new(ServerFrame::Call {
                    request_id: id.clone(),
                    operation,
                })))
                .is_err()
            {
                return reject_undispatched(&mut state, execution.as_ref(), "node channel full");
            }
            state.pending.insert(
                id.clone(),
                Pending {
                    node_id: node_id.into(),
                    generation,
                    tx,
                    execution,
                },
            );
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            _ => RpcReply::error(
                504,
                "node request timed out; retry using the same execution id",
            ),
        }
    }
}

fn release_claim(state: &mut State, record: &ExecutionIndex) -> Option<u64> {
    let (remove, remaining) = match state.reservations.get_mut(&record.id) {
        Some(reservation) if reservation.node_id == record.node_id => {
            reservation.claims = reservation.claims.saturating_sub(1);
            (reservation.claims == 0, reservation.claims)
        }
        _ => return None,
    };
    if remove {
        state.reservations.remove(&record.id);
    }
    if let Some(node) = state.nodes.get_mut(&record.node_id) {
        node.reserved_loops = node.reserved_loops.saturating_sub(1);
    }
    Some(remaining)
}

fn reject_undispatched(
    state: &mut State,
    execution: Option<&ExecutionIndex>,
    message: &str,
) -> RpcReply {
    if let Some(execution) = execution {
        let _ = release_claim(state, execution);
    }
    RpcReply::error(503, message)
}

#[cfg(test)]
#[path = "hub_tests.rs"]
mod tests;
