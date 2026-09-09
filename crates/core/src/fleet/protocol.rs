use super::{
    paging::{ArtifactRequest, DetailFieldRequest, EventPayloadRequest, MessageCursor},
    IndexReportEnvelope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// v7 pins named Harness profiles and registered binary Runner definitions.
pub const PROTOCOL_VERSION: u32 = 7;
pub const HEARTBEAT_MS: u64 = 5_000;
pub const STALE_MS: i64 = 20_000;
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    Agent,
    Dag,
    Team,
    Todos,
    Project,
    Maintenance,
    Operator,
    System,
}

impl ExecutionKind {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Dag => "dag",
            Self::Team => "team",
            Self::Todos => "todos",
            Self::Project => "project",
            Self::Maintenance => "maintenance",
            Self::Operator => "operator",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Idle,
    Cancelling,
    Interrupted,
    Done,
    Error,
    Cancelled,
}

impl ExecutionStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Cancelled)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Cancelling => "cancelling",
            Self::Interrupted => "interrupted",
            Self::Done => "done",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The entire persistent server-side execution record. Do not add detail fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionIndex {
    pub id: String,
    pub created_at: i64,
    pub kind: ExecutionKind,
    pub node_id: String,
    pub status: ExecutionStatus,
}

/// Stable identity used when the control plane asks the owning node for data.
/// The kind is explicit so neither side has to infer routing from an ID prefix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRef {
    pub id: String,
    pub kind: ExecutionKind,
}

impl ExecutionIndex {
    pub fn execution_ref(&self) -> ExecutionRef {
        ExecutionRef {
            id: self.id.clone(),
            kind: self.kind,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateExecution {
    pub id: String,
    pub kind: ExecutionKind,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub node_id: Option<String>,
}

impl CreateExecution {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.id) || !self.id.starts_with(&format!("{}-", self.kind.prefix())) {
            return Err(format!(
                "id must start with {}- and contain only letters, digits, '-' or '_'",
                self.kind.prefix()
            ));
        }
        if self.node_id.as_deref().is_some_and(|id| !valid_id(id)) {
            return Err("invalid node_id".into());
        }
        Ok(())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(default)]
    pub runtime: Option<Box<crate::harness::RuntimeSettings>>,
    #[serde(default)]
    pub codex: Option<Box<crate::harness::CodexSettings>>,
    pub index: ExecutionIndex,
    pub request: CreateExecution,
    #[serde(default)]
    pub definition: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCommand {
    pub action: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeAdmissionCommand {
    Freeze,
    Reopen,
    Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegistration {
    pub protocol_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub maintenance_agent_id: String,
    pub kinds: Vec<ExecutionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    #[serde(default)]
    pub pending_runs: u64,
    #[serde(default)]
    pub queue_order: super::QueueOrder,
    pub generation: String,
    pub sequence: u64,
    pub cpu_capacity: f64,
    pub active_agent_loops: u64,
    pub active_runs: u64,
    pub max_runs: u64,
    pub ready: bool,
    #[serde(default)]
    pub resource_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    #[serde(flatten)]
    pub registration: NodeRegistration,
    pub online: bool,
    pub last_seen_at: i64,
    pub snapshot: Option<NodeSnapshot>,
    pub reserved_loops: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum NodeOperation {
    Admission {
        command: NodeAdmissionCommand,
    },
    Create {
        assignment: Assignment,
    },
    Inspect {
        execution: ExecutionRef,
    },
    /// Return only the durable request receipt, without loading execution detail.
    AcceptedRequest {
        execution: ExecutionRef,
    },
    Command {
        execution: ExecutionRef,
        command: ExecutionCommand,
    },
    Events {
        execution: ExecutionRef,
        after: i64,
    },
    EventPayload {
        request: EventPayloadRequest,
    },
    DetailField {
        request: DetailFieldRequest,
    },
    Messages {
        execution: ExecutionRef,
        #[serde(default)]
        cursor: MessageCursor,
    },
    TodoItems {
        execution: ExecutionRef,
        #[serde(default)]
        after_ordinal: Option<i64>,
    },
    ProjectRuns {
        execution: ExecutionRef,
        #[serde(default)]
        before_version: Option<i64>,
    },
    TeamTurns {
        execution: ExecutionRef,
        #[serde(default)]
        after_turn: u32,
    },
    Artifact {
        request: ArtifactRequest,
    },
    /// Structured local maintenance; execution control uses Command with an ID.
    Maintenance {
        command: ExecutionCommand,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcReply {
    pub status: u16,
    pub body: Value,
}

impl RpcReply {
    pub fn ok(body: Value) -> Self {
        Self { status: 200, body }
    }
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            body: serde_json::json!({"error": message.into()}),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Call {
        request_id: String,
        operation: NodeOperation,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeFrame {
    Hello {
        registration: NodeRegistration,
        snapshot: NodeSnapshot,
    },
    Snapshot {
        snapshot: NodeSnapshot,
    },
    IndexReport {
        report: IndexReportEnvelope,
    },
    Reply {
        request_id: String,
        reply: RpcReply,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub id: String,
    pub agent: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDefinition {
    pub name: String,
    pub captain: String,
    pub members: Vec<TeamMember>,
}

impl TeamDefinition {
    pub fn validate(&self) -> Result<(), String> {
        let ids: std::collections::HashSet<_> = self.members.iter().map(|m| &m.id).collect();
        if !valid_id(&self.name)
            || !self.name.as_bytes()[0].is_ascii_alphanumeric()
            || self
                .name
                .bytes()
                .any(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
            || self.name == "system"
            || self.members.is_empty()
            || ids.len() != self.members.len()
            || !ids.contains(&self.captain)
            || self
                .members
                .iter()
                .any(|m| !valid_id(&m.id) || m.agent.trim().is_empty() || m.role.trim().is_empty())
        {
            return Err("team requires a unique member id, agent and role for each member, and a captain belonging to the team; system is reserved".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityTarget {
    pub kind: ExecutionKind,
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_reference_preserves_explicit_kind() {
        let index = ExecutionIndex {
            id: "agent-example".into(),
            created_at: 7,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Running,
        };

        assert_eq!(
            index.execution_ref(),
            ExecutionRef {
                id: "agent-example".into(),
                kind: ExecutionKind::Agent,
            }
        );
    }

    #[test]
    fn wire_index_requires_kind() {
        let missing_kind = serde_json::json!({
            "id": "agent-example",
            "created_at": 7,
            "node_id": "node-a",
            "status": "running"
        });
        assert!(serde_json::from_value::<ExecutionIndex>(missing_kind).is_err());

        let operation = NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-example".into(),
                kind: ExecutionKind::Agent,
            },
        };
        assert_eq!(
            serde_json::to_value(operation).unwrap(),
            serde_json::json!({
                "operation": "inspect",
                "execution": {"id": "agent-example", "kind": "agent"}
            })
        );
    }
}
