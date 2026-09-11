//! Playbook dispatch on the control plane: `POST /api/brain/playbooks/:id/dispatch`
//! expands one persisted playbook into per-step `CreateExecution` submits,
//! and `POST /api/brain/playbooks/trigger-scan` is the 一期 manual fallback
//! that reports which message-triggered playbooks would fire for one text
//! (auto-scanning inbound messages lands 二期).
//!
//! Dispatch mirrors `brain.rs::dispatch` conventions (RpcReply helpers,
//! request_id validation, idempotent replay keyed on the computed execution
//! ids, and a request_id fingerprint gate — the same key with different
//! content answers 409 instead of replaying the earlier run) and submits
//! through `executions::submit` so admission, placement and
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
use opencoder_brain::playbook::{
    self, PlaybookRouteKind, PlaybookSpec, PlaybookStep, PlaybookTarget,
};
use opencoder_core::fleet::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{error_400, error_404, error_500, response};
use crate::AppState;

/// Longest request key reused inside step execution ids. The key is used
/// VERBATIM — no truncation — so two distinct request ids can never share
/// an execution-id head; the cap keeps the id inside `valid_id`'s 64-byte
/// ceiling with room for the step-name tail (`valid_id` allows 64 bytes
/// total; the auto-generated ULID fallback is exactly 26 chars).
const MAX_KEY_CHARS: usize = 26;

/// Boundedness of the in-process idempotency gate: one slot per distinct
/// request_id, whole-map cap — a leaky client cannot grow it unbounded.
const MAX_DISPATCH_KEYS: usize = 512;

/// Request-scoped dispatch fingerprints: a request_id reused with a
/// different dispatch (playbook, situation or node) is a client bug and
/// answers 409 instead of silently replaying the earlier executions.
pub struct PlaybookGate {
    states: tokio::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl Default for PlaybookGate {
    fn default() -> Self {
        Self {
            states: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl PlaybookGate {
    /// Claim one request_id for a dispatch fingerprint. Same fingerprint
    /// (idempotent retry) claims again; a different fingerprint is a 409;
    /// a fresh key on a full map is a 503 (retry later, never an eviction).
    pub async fn claim(&self, request_id: &str, fingerprint: &str) -> Result<(), RpcReply> {
        let mut states = self.states.lock().await;
        if states
            .get(request_id)
            .is_some_and(|seen| seen != fingerprint)
        {
            return Err(RpcReply::error(
                409,
                "request_id already dispatched different content; use a new request_id",
            ));
        }
        if !states.contains_key(request_id) && states.len() >= MAX_DISPATCH_KEYS {
            return Err(RpcReply::error(
                503,
                "playbook idempotency gate is at capacity; retry later",
            ));
        }
        states.insert(request_id.to_string(), fingerprint.to_string());
        Ok(())
    }
}

/// Dispatch fingerprint: one digest over the canonical
/// `{playbook}\u{1}{situation}\u{1}{node}` tuple — a request_id reused with
/// any of those changed is a client bug, not an idempotent retry.
fn dispatch_fingerprint(id: &str, situation: &str, node_id: Option<&str>) -> String {
    opencoder_brain::situation_digest(&format!(
        "{id}\u{1}{situation}\u{1}{}",
        node_id.unwrap_or("")
    ))
}

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
        if !valid_id(request_id) {
            return error_400("invalid request_id".into());
        }
        // The key lands in every step execution id verbatim, so a longer
        // key would have to be truncated — which mints identical ids for
        // distinct prefixes. The cap makes that class of collision
        // unrepresentable instead.
        if request_id.len() > MAX_KEY_CHARS {
            return error_400(format!(
                "request_id must be at most {MAX_KEY_CHARS} chars to keep step execution ids collision-free"
            ));
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
    // An empty situation would silently substitute "" into every
    // `{situation}` placeholder and dispatch executions with an empty
    // prompt; playbooks whose prompts never reference the placeholder
    // dispatch fine without one.
    if situation.trim().is_empty()
        && spec
            .steps
            .iter()
            .any(|step| step.prompt.contains("{situation}"))
    {
        return error_400(
            "situation must not be empty when a step prompt references {situation}".into(),
        );
    }
    // Same request_id, different dispatch content (playbook, situation or
    // node) is a client bug: claim it before computing/replaying requests
    // so the earlier dispatch's executions are never presented as this
    // request's result. Invalid keys/specs above still win.
    if let Some(request_id) = body.request_id.as_deref() {
        let fingerprint = dispatch_fingerprint(&id, &situation, body.node_id.as_deref());
        if let Err(reply) = state.playbook_gate.claim(request_id, &fingerprint).await {
            return response(reply);
        }
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
/// every message trigger's `match_text` through the brain runtime — ONE
/// batched `embed_many` round-trip, not N+1 `embed_one` calls — and report
/// the playbooks whose trigger fires. Specs are parsed once here because
/// `trigger::scan` would re-decode every `spec_json` a second time; the
/// pure filter stays in the brain crate for its unit tests. An embed
/// outage is a 502 — the scan never silently pretends nothing matched.
/// Auto-scanning inbound messages lands 二期.
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
    // Parse each spec ONCE (corrupt rows → None, skipped — fail-open, a
    // corrupt row must not take the scan down).
    let parsed: Vec<_> = records
        .iter()
        .map(|record| {
            (
                record,
                serde_json::from_str::<PlaybookSpec>(&record.spec_json).ok(),
            )
        })
        .collect();
    // Incoming text first, then every parseable Message trigger's
    // match_text in record order; `embed_many` preserves input order.
    let texts: Vec<String> = std::iter::once(text.to_string())
        .chain(parsed.iter().filter_map(|(_, spec)| {
            spec.as_ref()
                .and_then(|spec| playbook::trigger::match_text(&spec.trigger).map(str::to_string))
        }))
        .collect();
    let embeddings = match state.brain.embed_many(&texts) {
        Ok(vecs) => vecs,
        Err(error) => return response(embed_error(error)),
    };
    let (incoming, match_embs) = embeddings.split_first().expect("input text always embeds");
    let mut match_embs = match_embs.iter();
    let mut matches = Vec::new();
    for (record, spec) in &parsed {
        let Some(spec) = spec else {
            continue; // corrupt rows are skipped, not fatal
        };
        // The iterator walks in lockstep with the chained match_texts
        // above, so every Message trigger pairs with its own embedding.
        let similarity = playbook::trigger::match_text(&spec.trigger).and_then(|_| {
            let emb = match_embs.next().expect("embed_many preserves input order");
            playbook::trigger::cosine_similarity(incoming, emb)
        });
        if playbook::trigger::fires(&spec.trigger, similarity) {
            matches.push(json!({
                "playbook_id": record.id,
                "name": record.name,
                "similarity": similarity,
            }));
        }
    }
    response(RpcReply::ok(json!({
        "ok": true,
        "matches": matches,
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
/// targets pass through; brain targets honor an inline route first and
/// otherwise resolve the control-plane `capability_target` binding (missing
/// binding → default agent "act", mirroring `brain.rs::plan`), and
/// non-executable kinds are a 400.
async fn resolve_target(
    state: &AppState,
    step: &PlaybookStep,
) -> Result<(ExecutionKind, String), RpcReply> {
    match &step.target {
        PlaybookTarget::Agent { agent } => Ok((ExecutionKind::Agent, agent.clone())),
        PlaybookTarget::Team { team } => Ok((ExecutionKind::Team, team.clone())),
        PlaybookTarget::Dag { dag } => Ok((ExecutionKind::Dag, dag.clone())),
        PlaybookTarget::Todos { workflow } => Ok((ExecutionKind::Todos, workflow.clone())),
        PlaybookTarget::Brain {
            capability_id,
            route,
        } => {
            // The inline route is the cross-end determinism channel: it
            // pins one executor for every end and overrides the
            // environment's `capability_target` binding. All three route
            // kinds are directly executable.
            if let Some(route) = route {
                return Ok(match route.kind {
                    PlaybookRouteKind::Agent => (ExecutionKind::Agent, route.ref_.clone()),
                    PlaybookRouteKind::Team => (ExecutionKind::Team, route.ref_.clone()),
                    PlaybookRouteKind::Dag => (ExecutionKind::Dag, route.ref_.clone()),
                });
            }
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
/// kind prefix is `CreateExecution::validate`'s contract). The key is used
/// VERBATIM — the caller guarantees `key.len() <= MAX_KEY_CHARS` (the
/// handler's request_id validation), so distinct keys can never share an id
/// head and the id stays inside `valid_id`'s 64-byte ceiling; when the step
/// name must shrink, a short hash of the full name keeps sibling steps
/// distinct.
fn step_execution_id(kind: ExecutionKind, key: &str, step: &str) -> String {
    debug_assert!(key.len() <= MAX_KEY_CHARS, "key must be pre-validated");
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
        // The key is embedded verbatim, so a full-budget (26-char) key is
        // recognizable in the id and distinct keys never collide.
        let key = "k".repeat(MAX_KEY_CHARS);
        let id = step_execution_id(ExecutionKind::Todos, &key, "fetch");
        assert!(id.starts_with(&format!("todos-pbk-{key}-")), "{id}");
        assert!(valid_id(&id), "{id}");
        let long = "s".repeat(80);
        let id = step_execution_id(ExecutionKind::Todos, &key, &long);
        assert!(valid_id(&id), "{id}");
        assert_ne!(
            id,
            step_execution_id(ExecutionKind::Todos, &key, &format!("{}x", long)),
            "hash tail keeps sibling steps distinct"
        );
    }

    #[tokio::test]
    async fn playbook_gate_replays_same_content_and_rejects_changes() {
        let gate = PlaybookGate::default();
        gate.claim("r1", "f1").await.unwrap();
        gate.claim("r1", "f1").await.unwrap(); // idempotent retry re-claims
        let conflict = gate.claim("r1", "f2").await.unwrap_err();
        assert_eq!(conflict.status, 409, "{:?}", conflict.body);
    }

    #[tokio::test]
    async fn playbook_gate_answers_503_at_capacity() {
        let gate = PlaybookGate::default();
        gate.claim("kept", "f").await.unwrap();
        // "kept" occupies slot 512 already, so exactly 511 more keys fit.
        for i in 0..MAX_DISPATCH_KEYS - 1 {
            gate.claim(&format!("k{i}"), "f").await.unwrap();
        }
        let full = gate.claim("overflow", "f").await.unwrap_err();
        assert_eq!(full.status, 503, "{:?}", full.body);
        // Known keys are never evicted: the same fingerprint still claims.
        gate.claim("kept", "f").await.unwrap();
    }

    #[test]
    fn dispatch_fingerprint_covers_playbook_situation_and_node() {
        let base = dispatch_fingerprint("pbk", "s1", Some("node-1"));
        assert_eq!(base, dispatch_fingerprint("pbk", "s1", Some("node-1")));
        assert_ne!(base, dispatch_fingerprint("pbk-2", "s1", Some("node-1")));
        assert_ne!(base, dispatch_fingerprint("pbk", "s2", Some("node-1")));
        assert_ne!(base, dispatch_fingerprint("pbk", "s1", None));
    }
}
