//! Pure planning helpers for freshness runs: no state, no I/O, unit-tested.
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use opencoder_ontology::{FreshnessItemStatus, FreshnessRunItem, FreshnessRunStatus};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Upper bound for the policy-declared fan-out width.
pub(super) const MAX_CONCURRENCY: usize = 30;
/// Bound for item error text kept in the run record.
pub(super) const MAX_ERROR_CHARS: usize = 300;
/// Bound for candidate summaries; mirrors the storage-side limit.
pub(super) const MAX_CANDIDATE_CHARS: usize = 8000;

/// Status, optional candidate and optional error for one item, in that order.
pub type ItemOutcome = (FreshnessItemStatus, Option<String>, Option<String>);

/// Lowercase hex without an extra dependency.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Deterministic execution id: `<kind>-<24 hex chars>` of the SHA-256 over
/// `freshness/<run id>/<entity id>`. Stable across retries and compliant
/// with the fleet id rules (`valid_id`, kind prefix).
pub fn execution_id(kind_prefix: &str, run_id: Uuid, entity_id: Uuid) -> String {
    let digest = Sha256::digest(format!("freshness/{run_id}/{entity_id}").as_bytes());
    format!("{kind_prefix}-{}", &hex(&digest)[..24])
}

/// Substitute `{{entity.<field>}}` placeholders. Known fields are id, name,
/// description and data_level; unknown fields and missing values render as
/// empty strings so templates never leak raw placeholders.
pub fn render_prompt(template: &str, entity: &Value) -> String {
    const OPEN: &str = "{{entity.";
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        rest = &rest[start + OPEN.len()..];
        match rest.find("}}") {
            Some(end) => {
                let key = &rest[..end];
                out.push_str(match key {
                    "id" | "name" | "description" | "data_level" => {
                        entity[key].as_str().unwrap_or_default()
                    }
                    _ => "",
                });
                rest = &rest[end + 2..];
            }
            None => {
                out.push_str(OPEN);
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Executor kind declared by the policy; anything but `dag` means agent.
pub fn executor_kind(policy: &Value) -> ExecutionKind {
    if policy["executor"]["kind"] == "dag" {
        ExecutionKind::Dag
    } else {
        ExecutionKind::Agent
    }
}

/// Frozen per-entity execution input. Agent executions carry the rendered
/// prompt (or a name/description fallback when the template is blank); DAG
/// executions additionally snapshot the entity and keep a null prompt when
/// nothing was rendered.
pub fn execution_input(policy: &Value, entity: &Value) -> Value {
    let template = policy["executor"]["prompt_template"]
        .as_str()
        .unwrap_or_default();
    let rendered = (!template.trim().is_empty()).then(|| render_prompt(template, entity));
    if executor_kind(policy) == ExecutionKind::Dag {
        return serde_json::json!({ "entity": entity, "prompt": rendered });
    }
    match rendered {
        Some(prompt) => serde_json::json!({ "prompt": prompt }),
        None => {
            let name = entity["name"].as_str().unwrap_or_default();
            let description = entity["description"].as_str().unwrap_or_default();
            serde_json::json!({ "prompt": format!("Refresh entity {name}: {description}") })
        }
    }
}

/// Executor target; blank values are treated as unplaced.
pub fn executor_target(policy: &Value) -> Option<String> {
    policy["executor"]["target"]
        .as_str()
        .filter(|target| !target.trim().is_empty())
        .map(str::to_owned)
}

/// Fan-out width from the policy, clamped to 1..=MAX_CONCURRENCY.
pub fn concurrency(policy: &Value) -> usize {
    policy["concurrency"]
        .as_u64()
        .unwrap_or(1)
        .clamp(1, MAX_CONCURRENCY as u64) as usize
}

/// Character-bound a message for storage.
pub fn bound(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Failed outcome factory shared by submit, poll and reconcile paths.
pub(super) fn failed_outcome(text: &str) -> ItemOutcome {
    (
        FreshnessItemStatus::Failed,
        None,
        Some(bound(text, MAX_ERROR_CHARS)),
    )
}

/// Terminal run status derived from the item mix. Mirrors the data-plane
/// rule exactly (empty or successless → failed, failureless → succeeded,
/// else partial) so terminal saves are never rejected.
pub fn settle(items: &[FreshnessRunItem]) -> FreshnessRunStatus {
    match (
        items.is_empty(),
        items
            .iter()
            .any(|item| item.status == FreshnessItemStatus::Failed),
        items
            .iter()
            .any(|item| item.status == FreshnessItemStatus::Succeeded),
    ) {
        (true, _, _) => FreshnessRunStatus::Failed,
        (_, false, _) => FreshnessRunStatus::Succeeded,
        (_, _, false) => FreshnessRunStatus::Failed,
        _ => FreshnessRunStatus::Partial,
    }
}

/// Legal run transitions as enforced by the data plane: queued starts,
/// running finishes, terminals are final.
pub fn transition_allowed(current: FreshnessRunStatus, next: FreshnessRunStatus) -> bool {
    matches!(
        (current, next),
        (FreshnessRunStatus::Queued, FreshnessRunStatus::Running)
            | (
                FreshnessRunStatus::Running,
                FreshnessRunStatus::Succeeded
                    | FreshnessRunStatus::Partial
                    | FreshnessRunStatus::Failed
            )
    )
}

/// Item progress order: pending → dispatched → terminal.
fn progressed(stored: FreshnessItemStatus, mine: FreshnessItemStatus) -> bool {
    matches!(
        (stored, mine),
        (
            FreshnessItemStatus::Pending,
            FreshnessItemStatus::Dispatched
                | FreshnessItemStatus::Succeeded
                | FreshnessItemStatus::Failed
        ) | (
            FreshnessItemStatus::Dispatched,
            FreshnessItemStatus::Succeeded | FreshnessItemStatus::Failed
        )
    )
}

/// Copy local progress onto freshly read items; identity is the entity id
/// and statuses never move backwards, so stored items ahead of the local
/// state keep theirs.
pub fn merge_progress(stored: &mut [FreshnessRunItem], local: &[FreshnessRunItem]) {
    for item in stored.iter_mut() {
        let Some(mine) = local
            .iter()
            .find(|mine| mine.entity_id == item.entity_id)
            .filter(|mine| progressed(item.status, mine.status))
        else {
            continue;
        };
        item.status = mine.status;
        if mine.execution_id.is_some() {
            item.execution_id.clone_from(&mine.execution_id);
        }
        if matches!(
            mine.status,
            FreshnessItemStatus::Succeeded | FreshnessItemStatus::Failed
        ) {
            item.candidate.clone_from(&mine.candidate);
            item.error.clone_from(&mine.error);
        }
    }
}

/// Outcome of inspecting a dispatched execution's fleet record.
pub enum PollOutcome {
    /// Not terminal yet; the item stays dispatched.
    Pending,
    /// Terminal success; the result summary still has to be read.
    Done,
    /// Terminal failure with the mapped outcome.
    Failed(ItemOutcome),
}

/// Map a fleet execution status onto a poll outcome.
pub fn poll_from_index(status: ExecutionStatus) -> PollOutcome {
    match status {
        ExecutionStatus::Done => PollOutcome::Done,
        ExecutionStatus::Error => PollOutcome::Failed(failed_outcome("execution failed")),
        ExecutionStatus::Cancelled => PollOutcome::Failed(failed_outcome("execution cancelled")),
        _ => PollOutcome::Pending,
    }
}

/// Map an on-demand result read onto an item outcome. A readable reply
/// settles the item as succeeded (the summary is an optional candidate);
/// an unreadable one fails it with the bounded error text.
pub fn outcome_from_read(status: u16, body: &Value) -> ItemOutcome {
    if status < 300 {
        let candidate = body["summary"]
            .as_str()
            .map(|summary| bound(summary, MAX_CANDIDATE_CHARS));
        (FreshnessItemStatus::Succeeded, candidate, None)
    } else {
        failed_outcome(&format!("result {status}: {body}"))
    }
}

/// Overwrite an item with a computed outcome.
pub(super) fn apply(item: &mut FreshnessRunItem, (status, candidate, error): ItemOutcome) {
    item.status = status;
    item.candidate = candidate;
    item.error = error;
}

#[cfg(test)]
#[path = "planning_tests.rs"]
mod tests;
