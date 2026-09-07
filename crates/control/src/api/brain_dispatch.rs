use opencoder_core::fleet::{valid_id, CreateExecution, ExecutionIndex, ExecutionKind, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::{Mutex, MutexGuard};

use crate::api_brain::{DispatchBody, DEFAULT_SEARCH_K, MAX_SEARCH_K};

pub const RECEIPT_KEY: &str = "brain_receipt";
const RECEIPT_VERSION: u8 = 1;
const SHARDS: usize = 64;
const MAX_KEYS_PER_SHARD: usize = 128;

pub const ROUTABLE_KINDS: [ExecutionKind; 4] = [
    ExecutionKind::Agent,
    ExecutionKind::Team,
    ExecutionKind::Dag,
    ExecutionKind::Todos,
];

#[derive(Debug, Clone, Deserialize)]
struct WireRequest {
    situation: String,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    top_k: Option<u32>,
    #[serde(default)]
    replan: Option<bool>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalIntent {
    pub situation: String,
    pub node_id: Option<String>,
    pub plan_id: Option<String>,
    pub top_k: u32,
    pub replan: bool,
    /// The client override only. A changed server default is not a new request.
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NormalizedRequest {
    pub request_id: Option<String>,
    pub custom_id: Option<String>,
    pub intent: CanonicalIntent,
}

impl NormalizedRequest {
    pub fn preview(&self) -> DispatchBody {
        DispatchBody {
            situation: self.intent.situation.clone(),
            plan_id: self.intent.plan_id.clone(),
            top_k: Some(self.intent.top_k),
            replan: Some(self.intent.replan),
            model: self.intent.model.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainDecision {
    pub plan_id: Option<String>,
    pub capability_id: Option<String>,
    pub reason: Value,
    pub path: Value,
    pub planned_fresh: bool,
    pub planner_model: String,
    pub kind: ExecutionKind,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrainReceipt {
    pub version: u8,
    pub request_id: String,
    pub fingerprint: String,
    pub intent: CanonicalIntent,
    pub decision: BrainDecision,
}

#[derive(Debug, Clone)]
pub struct PreparedDispatch {
    pub request: CreateExecution,
    pub receipt: BrainReceipt,
}

#[derive(Debug, Clone)]
pub struct KeyState {
    intent: CanonicalIntent,
    pub prepared: Option<PreparedDispatch>,
}

pub struct BrainGate {
    shards: [Mutex<HashMap<String, KeyState>>; SHARDS],
}

impl Default for BrainGate {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new(HashMap::new())),
        }
    }
}

impl BrainGate {
    pub async fn lock(&self, request_id: &str) -> MutexGuard<'_, HashMap<String, KeyState>> {
        self.shards[shard(request_id)].lock().await
    }
}

fn shard(value: &str) -> usize {
    value
        .bytes()
        .fold(0usize, |hash, byte| hash.wrapping_mul(31) ^ byte as usize)
        % SHARDS
}

pub fn normalize(body: &Value) -> Result<NormalizedRequest, RpcReply> {
    let wire: WireRequest = serde_json::from_value(body.clone())
        .map_err(|error| RpcReply::error(400, error.to_string()))?;
    let situation = wire.situation.trim().to_string();
    if situation.is_empty() {
        return Err(RpcReply::error(400, "situation must not be empty"));
    }
    if let Some(request_id) = wire.request_id.as_deref() {
        if !valid_id(request_id) || request_id.len() > 48 {
            return Err(RpcReply::error(400, "invalid request_id"));
        }
        if body.get("id").is_some() {
            return Err(RpcReply::error(
                400,
                "id cannot be combined with request_id",
            ));
        }
    }
    if wire.node_id.as_deref().is_some_and(|id| !valid_id(id)) {
        return Err(RpcReply::error(400, "invalid node_id"));
    }
    Ok(NormalizedRequest {
        request_id: wire.request_id,
        custom_id: wire.id,
        intent: CanonicalIntent {
            situation,
            node_id: wire.node_id,
            plan_id: trimmed(wire.plan_id),
            top_k: wire
                .top_k
                .unwrap_or(DEFAULT_SEARCH_K)
                .clamp(1, MAX_SEARCH_K),
            replan: wire.replan.unwrap_or(false),
            model: trimmed(wire.model),
        },
    })
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn fingerprint(intent: &CanonicalIntent) -> String {
    let canonical = serde_json::to_string(intent).expect("canonical intent is serializable");
    opencoder_brain::situation_digest(&canonical)
}

pub fn candidate_ids(request_id: &str) -> [(ExecutionKind, String); 4] {
    ROUTABLE_KINDS.map(|kind| (kind, format!("{}-{request_id}", kind.prefix())))
}

pub fn claim<'a>(
    states: &'a mut HashMap<String, KeyState>,
    request_id: &str,
    intent: &CanonicalIntent,
) -> Result<&'a mut KeyState, RpcReply> {
    if states
        .get(request_id)
        .is_some_and(|state| state.intent != *intent)
    {
        return Err(conflict());
    }
    if !states.contains_key(request_id) && states.len() >= MAX_KEYS_PER_SHARD {
        return Err(RpcReply::error(
            503,
            "brain idempotency gate is at capacity; retry later",
        ));
    }
    Ok(states.entry(request_id.to_string()).or_insert(KeyState {
        intent: intent.clone(),
        prepared: None,
    }))
}

pub fn prepare(
    normalized: &NormalizedRequest,
    request_id: &str,
    result: &Value,
    kind: ExecutionKind,
    target: String,
    planner_model: String,
) -> Result<PreparedDispatch, RpcReply> {
    if !ROUTABLE_KINDS.contains(&kind) || target.trim().is_empty() {
        return Err(RpcReply::error(400, "brain target is not executable"));
    }
    let string = |key: &str| {
        result[key]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| RpcReply::error(500, format!("brain result missing {key}")))
    };
    let direct = result["route"].as_str() == Some("default_agent");
    if direct && (kind != ExecutionKind::Agent || target != "act") {
        return Err(RpcReply::error(500, "invalid default brain route"));
    }
    let decision = BrainDecision {
        plan_id: if direct {
            None
        } else {
            Some(string("plan_id")?)
        },
        capability_id: if direct {
            None
        } else {
            Some(string("capability_id")?)
        },
        reason: result["reason"].clone(),
        path: result["path"].clone(),
        planned_fresh: result["planned_fresh"].as_bool().unwrap_or(false),
        planner_model,
        kind,
        target: target.clone(),
    };
    let receipt = BrainReceipt {
        version: RECEIPT_VERSION,
        request_id: request_id.to_string(),
        fingerprint: fingerprint(&normalized.intent),
        intent: normalized.intent.clone(),
        decision,
    };
    let mut input = json!({
        "prompt": normalized.intent.situation,
        "capability_id": receipt.decision.capability_id,
    });
    input[RECEIPT_KEY] = json!(receipt);
    let request = CreateExecution {
        id: format!("{}-{request_id}", kind.prefix()),
        kind,
        target: Some(target),
        input,
        node_id: normalized.intent.node_id.clone(),
    };
    Ok(PreparedDispatch { request, receipt })
}

pub fn receipt_from_accepted(
    body: &Value,
    index: &ExecutionIndex,
    request_id: &str,
    intent: &CanonicalIntent,
) -> Result<BrainReceipt, RpcReply> {
    if body["id"].as_str() != Some(index.id.as_str())
        || serde_json::from_value::<ExecutionKind>(body["kind"].clone()).ok() != Some(index.kind)
    {
        return Err(RpcReply::error(409, "accepted execution identity mismatch"));
    }
    let receipt: BrainReceipt = serde_json::from_value(body["receipt"].clone())
        .map_err(|_| RpcReply::error(409, "execution was not created by brain dispatch"))?;
    let expected_fingerprint = fingerprint(intent);
    if receipt.version != RECEIPT_VERSION
        || receipt.request_id != request_id
        || receipt.intent != *intent
        || receipt.fingerprint != expected_fingerprint
        || self::fingerprint(&receipt.intent) != receipt.fingerprint
        || receipt.decision.kind != index.kind
        || !ROUTABLE_KINDS.contains(&receipt.decision.kind)
        || receipt.decision.target.trim().is_empty()
        || receipt.decision.plan_id.is_some() != receipt.decision.capability_id.is_some()
        || (receipt.decision.capability_id.is_none()
            && (receipt.decision.kind != ExecutionKind::Agent || receipt.decision.target != "act"))
    {
        return Err(conflict());
    }
    Ok(receipt)
}

pub fn dispatch_reply(index: &ExecutionIndex, receipt: &BrainReceipt) -> RpcReply {
    RpcReply {
        status: 202,
        body: json!({
            "ok": true,
            "route": if receipt.decision.capability_id.is_some() { "capability" } else { "default_agent" },
            "plan_id": receipt.decision.plan_id,
            "capability_id": receipt.decision.capability_id,
            "reason": receipt.decision.reason,
            "path": receipt.decision.path,
            "planned_fresh": receipt.decision.planned_fresh,
            "execution": index,
        }),
    }
}

pub fn conflict() -> RpcReply {
    RpcReply::error(409, "request_id already used with different input")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_defaults_and_candidates_are_stable() {
        let a = normalize(&json!({"situation":" x ","request_id":"r"})).unwrap();
        let b = normalize(&json!({
            "situation":"x","request_id":"r","top_k":10,"replan":false,"model":" "
        }))
        .unwrap();
        assert_eq!(a.intent, b.intent);
        assert_eq!(candidate_ids("r")[3].1, "todos-r");
        assert_eq!(fingerprint(&a.intent), fingerprint(&b.intent));
    }

    #[test]
    fn request_id_rejects_custom_id_and_changed_semantics() {
        assert_eq!(
            normalize(&json!({"situation":"x","request_id":"r","id":null}))
                .unwrap_err()
                .status,
            400
        );
        let intent = normalize(&json!({"situation":"x","request_id":"r"})).unwrap();
        let mut states = HashMap::new();
        claim(&mut states, "r", &intent.intent).unwrap();
        let changed = normalize(&json!({"situation":"x","request_id":"r","top_k":11})).unwrap();
        assert_eq!(
            claim(&mut states, "r", &changed.intent).unwrap_err().status,
            409
        );
    }
}
