//! Playbook dispatch on the control plane: `POST /api/brain/playbooks/:id/dispatch`
//! expands one persisted playbook into per-step `CreateExecution` submits,
//! and `POST /api/brain/playbooks/trigger-scan` is the 一期 manual fallback
//! that reports which message-triggered playbooks would fire for one text
//! (auto-scanning inbound messages lands 二期).
//!
//! Dispatch mirrors `brain.rs::dispatch` conventions (RpcReply helpers,
//! request_id validation, idempotent replay keyed on the computed execution
//! ids) and submits through `executions::submit` so admission, placement and
//! node acceptance stay identical to every other execution path. Batch
//! submission is one-shot: 一期串行批次提交，无跨请求等待图 — the topological
//! batches are submitted back-to-back and only reported in the response;
//! waiting on step completion is a later wave.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_brain::playbook::{self, PlaybookSpec, PlaybookStep, PlaybookTarget};
use opencoder_core::fleet::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{error_400, error_404, error_500, response};
use crate::AppState;

/// Longest request key reused inside step execution ids (`valid_id` allows
/// 64 bytes total; capping the key keeps room for the step-name tail).
const MAX_KEY_CHARS: usize = 26;

#[derive(Debug, Default, Deserialize)]
pub struct DispatchBody {
    /// Idempotency key: execution ids (and therefore the replay probe) are
    /// derived from it, so a retried dispatch resubmits nothing.
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    /// Substituted into every step prompt's `{situation}` placeholder.
    #[serde(default)]
    pub situation: Option<String>,
    /// Accepted for forward-compat (dynamic re-planning, 二期); a stored
    /// playbook dispatch never searches the capability library.
    #[serde(default)]
    pub top_k: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct TriggerScanBody {
    pub text: String,
}

pub async fn dispatch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<DispatchBody>,
) -> Response {
    if let Some(request_id) = body.request_id.as_deref() {
        if !valid_id(request_id) || request_id.len() > 48 {
            return error_400("invalid request_id".into());
        }
    }
    if body.node_id.as_deref().is_some_and(|node| !valid_id(node)) {
        return error_400("invalid node_id".into());
    }
    let _ = body.top_k;
    let situation = body.situation.unwrap_or_default();
    let spec = match state.brain.get_playbook_spec(&id).await {
        Ok(Some(spec)) => spec,
        Ok(None) => return error_404(&format!("brain playbook not found: {id}")),
        Err(error) => return error_500(error.to_string()),
    };
    if let Err(errs) = playbook::validate(&spec) {
        return error_500(format!("stored playbook invalid: {}", errs.join("; ")));
    }

    let batches = topo_batches(&spec);
    let mut plans = Vec::new();
    for step in batches.iter().flatten() {
        match resolve_target(&state, step).await {
            Ok((kind, target)) => plans.push(Plan { step, kind, target }),
            Err(reply) => return response(reply),
        }
    }

    // The request key makes every step id deterministic per request: a
    // retry with the same request_id recomputes the same ids.
    let key = body
        .request_id
        .clone()
        .unwrap_or_else(|| ulid::Ulid::new().to_string());
    let requests: Vec<CreateExecution> = plans
        .iter()
        .map(|plan| CreateExecution {
            id: step_execution_id(plan.kind, &key, &plan.step.name),
            kind: plan.kind,
            target: Some(plan.target.clone()),
            input: json!({
                "prompt": playbook::render_prompt(&plan.step.prompt, &situation),
                "playbook_id": id,
                "step": plan.step.name,
            }),
            node_id: body.node_id.clone(),
        })
        .collect();

    // Idempotent replay (一期: request-scoped): every computed id already in
    // the execution index ⇒ the earlier dispatch landed; return the same
    // response shape without resubmitting anything.
    if body.request_id.is_some() && all_indexed(&state, &requests).await {
        let executions: Vec<Value> = plans
            .iter()
            .zip(&requests)
            .map(|(plan, request)| execution_json(&plan.step.name, plan.kind, &request.id))
            .collect();
        return response(dispatch_reply(&id, &batches, executions));
    }

    let mut executions = Vec::new();
    for (plan, request) in plans.iter().zip(requests) {
        let id = request.id.clone();
        let reply = super::executions::submit(&state, request).await;
        if reply.status != 202 {
            return response(reply);
        }
        executions.push(execution_json(&plan.step.name, plan.kind, &id));
    }
    response(dispatch_reply(&id, &batches, executions))
}

/// 一期 manual fallback for message triggers: embed the inbound text plus
/// every message trigger's `match_text` through the brain runtime and report
/// the playbooks whose trigger fires (`trigger::scan` does the pure filter).
/// An embed outage is a 502 — the scan never silently pretends nothing
/// matched. Auto-scanning inbound messages lands 二期.
pub async fn trigger_scan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TriggerScanBody>,
) -> Response {
    let text = body.text.trim();
    if text.is_empty() {
        return error_400("text must not be empty".into());
    }
    let records = match state.store.list_brain_playbooks().await {
        Ok(records) => records,
        Err(error) => return error_500(error.to_string()),
    };
    let incoming = match state.brain.embed_one(text) {
        Ok(emb) => emb,
        Err(error) => return response(embed_error(error)),
    };
    let mut similarities: BTreeMap<String, Option<f64>> = BTreeMap::new();
    for record in &records {
        let Ok(spec) = serde_json::from_str::<PlaybookSpec>(&record.spec_json) else {
            continue; // corrupt rows are skipped, not fatal
        };
        let Some(match_text) = playbook::trigger::match_text(&spec.trigger) else {
            continue; // manual triggers never auto-fire
        };
        let emb = match state.brain.embed_one(match_text) {
            Ok(emb) => emb,
            Err(error) => return response(embed_error(error)),
        };
        similarities.insert(
            record.id.clone(),
            playbook::trigger::cosine_similarity(&incoming, &emb),
        );
    }
    let matched = playbook::trigger::scan(&records, |record| {
        similarities.get(&record.id).copied().flatten()
    });
    response(RpcReply::ok(json!({
        "ok": true,
        "matches": matched
            .iter()
            .map(|record| json!({
                "playbook_id": record.id,
                "name": record.name,
                "similarity": similarities.get(&record.id).cloned().flatten(),
            }))
            .collect::<Vec<_>>(),
    })))
}

struct Plan<'a> {
    step: &'a PlaybookStep,
    kind: ExecutionKind,
    target: String,
}

/// Topological batches of a validated spec: batch 0 holds the roots, batch
/// n+1 the steps whose dependencies all sit in earlier batches (`ready_steps`
/// over a growing done-set; lexicographic order keeps it deterministic).
fn topo_batches(spec: &PlaybookSpec) -> Vec<Vec<&PlaybookStep>> {
    let by_name: BTreeMap<&str, &PlaybookStep> = spec
        .steps
        .iter()
        .map(|step| (step.name.as_str(), step))
        .collect();
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut batches = Vec::new();
    while done.len() < spec.steps.len() {
        let names = playbook::ready_steps(spec, &done);
        if names.is_empty() {
            break; // unreachable post-validate (cycles reject earlier)
        }
        for name in &names {
            done.insert(name.clone());
        }
        batches.push(names.iter().map(|name| by_name[name.as_str()]).collect());
    }
    batches
}

/// Map one step's target onto an executable (kind, target) pair. Direct
/// targets pass through; brain targets resolve the control-plane
/// `capability_target` binding (missing binding → default agent "act",
/// mirroring `brain.rs::plan`), and non-executable kinds are a 400.
async fn resolve_target(
    state: &AppState,
    step: &PlaybookStep,
) -> Result<(ExecutionKind, String), RpcReply> {
    match &step.target {
        PlaybookTarget::Agent { agent } => Ok((ExecutionKind::Agent, agent.clone())),
        PlaybookTarget::Team { team } => Ok((ExecutionKind::Team, team.clone())),
        PlaybookTarget::Dag { dag } => Ok((ExecutionKind::Dag, dag.clone())),
        PlaybookTarget::Todos { workflow } => Ok((ExecutionKind::Todos, workflow.clone())),
        PlaybookTarget::Brain { capability_id } => {
            let target: CapabilityTarget = match state
                .fleet
                .definition("capability_target", capability_id)
                .await
            {
                Ok(Some(value)) => serde_json::from_value(value)
                    .map_err(|error| RpcReply::error(500, error.to_string()))?,
                Ok(None) => CapabilityTarget {
                    kind: ExecutionKind::Agent,
                    target: "act".into(),
                },
                Err(error) => return Err(RpcReply::error(500, error.to_string())),
            };
            if !matches!(
                target.kind,
                ExecutionKind::Agent
                    | ExecutionKind::Team
                    | ExecutionKind::Dag
                    | ExecutionKind::Todos
            ) {
                return Err(RpcReply::error(400, "capability target is not executable"));
            }
            Ok((target.kind, target.target))
        }
    }
}

/// Deterministic per-request execution id: `{kind}-pbk-{key}-{step}` (the
/// kind prefix is `CreateExecution::validate`'s contract). The key is capped
/// so the id stays inside `valid_id`'s 64-byte ceiling; when the step name
/// must shrink, a short hash of the full name keeps sibling steps distinct.
fn step_execution_id(kind: ExecutionKind, key: &str, step: &str) -> String {
    let key = &key[..key.len().min(MAX_KEY_CHARS)];
    let head = format!("{}-pbk-{}-", kind.prefix(), key);
    let budget = 64usize.saturating_sub(head.len());
    let tail = if step.len() <= budget {
        step.to_string()
    } else {
        let hash = format!("{:08x}", fnv1a(step.as_bytes()));
        let keep = budget.saturating_sub(hash.len() + 1);
        let mut cut = keep.min(step.len());
        while cut > 0 && !step.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}-{hash}", &step[..cut])
    };
    format!("{head}{tail}")
}

fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |hash, byte| {
        hash.wrapping_mul(0x0100_0193)
            .wrapping_add(u32::from(*byte))
    })
}

async fn all_indexed(state: &AppState, requests: &[CreateExecution]) -> bool {
    for request in requests {
        match state.fleet.index(&request.id).await {
            Ok(Some(index)) if index.kind == request.kind => {}
            _ => return false,
        }
    }
    !requests.is_empty()
}

fn execution_json(step: &str, kind: ExecutionKind, id: &str) -> Value {
    json!({ "step": step, "kind": kind, "id": id })
}

fn dispatch_reply(id: &str, batches: &[Vec<&PlaybookStep>], executions: Vec<Value>) -> RpcReply {
    RpcReply {
        status: 202,
        body: json!({
            "ok": true,
            "playbook_id": id,
            "batches": batches
                .iter()
                .map(|batch| batch.iter().map(|step| step.name.clone()).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            "executions": executions,
        }),
    }
}

fn embed_error(error: anyhow::Error) -> RpcReply {
    if error
        .downcast_ref::<opencoder_brain::EmbeddingFailed>()
        .is_some()
    {
        RpcReply::error(502, format!("embedding failed: {error:#}"))
    } else {
        RpcReply::error(500, error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_brain::{PlaybookOrigin, PlaybookTrigger};

    fn spec(steps: Vec<PlaybookStep>) -> PlaybookSpec {
        PlaybookSpec {
            schema_version: playbook::SCHEMA_VERSION,
            id: "playbook-t".into(),
            name: "t".into(),
            origin: PlaybookOrigin::Fixed {},
            trigger: PlaybookTrigger::Manual {},
            steps,
        }
    }

    fn step(name: &str, deps: &[&str]) -> PlaybookStep {
        PlaybookStep {
            name: name.into(),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            target: PlaybookTarget::Agent {
                agent: "act".into(),
            },
            prompt: "p {situation}".into(),
        }
    }

    #[test]
    fn batches_follow_the_dependency_layers() {
        let spec = spec(vec![
            step("a", &[]),
            step("c", &["b1", "b2"]),
            step("b2", &["a"]),
            step("b1", &["a"]),
        ]);
        let batches: Vec<Vec<&str>> = topo_batches(&spec)
            .iter()
            .map(|b| b.iter().map(|s| s.name.as_str()).collect())
            .collect();
        assert_eq!(batches, vec![vec!["a"], vec!["b1", "b2"], vec!["c"]]);
    }

    #[test]
    fn step_ids_are_prefixed_deterministic_and_length_safe() {
        let base = step_execution_id(ExecutionKind::Agent, "req-1", "fetch");
        assert_eq!(base, "agent-pbk-req-1-fetch");
        assert_eq!(
            base,
            step_execution_id(ExecutionKind::Agent, "req-1", "fetch")
        );
        assert!(valid_id(&base));
        let long = "s".repeat(80);
        let id = step_execution_id(ExecutionKind::Todos, &"k".repeat(48), &long);
        assert!(valid_id(&id), "{id}");
        assert_ne!(
            id,
            step_execution_id(ExecutionKind::Todos, &"k".repeat(48), &format!("{}x", long)),
            "hash tail keeps sibling steps distinct"
        );
    }
}
