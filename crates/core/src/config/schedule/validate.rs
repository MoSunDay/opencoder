//! Structural validation helpers for `schedules.json` jobs: id charset,
//! kind/target contracts and the brain params pre-check. Split from
//! `schedule.rs` to keep each file under the 400-line rule.

use serde_json::Value;

use super::{ScheduleJob, ScheduleKind, MAX_SCHEDULE_ID_LEN};

/// Schedule ids end up embedded in deterministic execution ids
/// (`<kind>-<id>-<fire ms>`), so they must be filesystem/URL safe.
pub fn validate_id(id: &str) -> Result<(), String> {
    let first = id.bytes().next();
    let valid = first.is_some_and(|b| b.is_ascii_alphanumeric())
        && id.len() <= MAX_SCHEDULE_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        return Ok(());
    }
    Err(format!(
        "schedule id {id:?} must be 1-{MAX_SCHEDULE_ID_LEN} chars of letters, digits, '-' or '_' \
         (it is embedded into deterministic execution ids)"
    ))
}

pub(super) fn validate_target(job: &ScheduleJob) -> Result<(), String> {
    let target = job.target.trim();
    match job.kind {
        ScheduleKind::Todos => {
            let (name, version) = target.split_once('/').ok_or_else(|| {
                format!("schedule {}: todos target must be template/version", job.id)
            })?;
            crate::validate_share_name(name).map_err(|e| format!("schedule {}: {e}", job.id))?;
            crate::validate_share_name(version).map_err(|e| format!("schedule {}: {e}", job.id))?;
        }
        ScheduleKind::Brain => {
            if !crate::fleet::valid_id(target) {
                return Err(format!(
                    "schedule {}: brain target must be a plan-def id (letters, digits, '-' or '_')",
                    job.id
                ));
            }
        }
        // team/agent/dag targets are fleet definition ids / agent names.
        ScheduleKind::Team | ScheduleKind::Agent | ScheduleKind::Dag => {
            if !crate::fleet::valid_id(target) {
                return Err(format!(
                    "schedule {}: {} target must be letters, digits, '-' or '_'",
                    job.id,
                    job.kind.as_str()
                ));
            }
        }
    }
    if !job.params.is_empty()
        && !matches!(
            job.kind,
            ScheduleKind::Agent | ScheduleKind::Team | ScheduleKind::Todos | ScheduleKind::Brain
        )
    {
        return Err(format!(
            "schedule {}: params are not supported for {}",
            job.id,
            job.kind.as_str()
        ));
    }
    Ok(())
}

/// brain params mirror `scheduler::brain_run`'s fire-time contract so a bad
/// config fails at config time instead of landing as an `error` ledger row
/// that retries for an hour: `objective` (required non-empty string),
/// `mode` (`fixed`|`dynamic`), `inputs` (object), `plan` (object with
/// string `id` + u64 `version`).
pub(super) fn validate_brain_params(job: &ScheduleJob) -> Result<(), String> {
    match job.params.get("objective") {
        Some(Value::String(text)) if !text.trim().is_empty() => {}
        _ => {
            return Err(format!(
                "schedule {}: brain params need a non-empty string `objective`",
                job.id
            ))
        }
    }
    if let Some(mode) = job.params.get("mode") {
        if mode.as_str() != Some("fixed") && mode.as_str() != Some("dynamic") {
            return Err(format!(
                "schedule {}: brain mode must be \"fixed\" or \"dynamic\" (a wrong value would \
                 silently fall back to fixed at fire time)",
                job.id
            ));
        }
    }
    if let Some(inputs) = job.params.get("inputs") {
        if !inputs.is_object() {
            return Err(format!(
                "schedule {}: brain inputs must be an object",
                job.id
            ));
        }
    }
    match job.params.get("plan") {
        None | Some(Value::Null) => {}
        Some(Value::Object(plan)) => {
            if !plan.get("id").is_some_and(Value::is_string) {
                return Err(format!(
                    "schedule {}: brain plan.id must be a string",
                    job.id
                ));
            }
            if !plan.get("version").is_some_and(Value::is_u64) {
                return Err(format!(
                    "schedule {}: brain plan.version must be a u64",
                    job.id
                ));
            }
        }
        Some(_) => return Err(format!("schedule {}: brain plan must be an object", job.id)),
    }
    Ok(())
}

/// Fixed validation instant: time templates must not depend on "when", so
/// the dry-run uses a pinned instant for reproducible errors.
pub(crate) fn base_instant() -> chrono::DateTime<chrono::FixedOffset> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("fixed instant")
        .fixed_offset()
}

pub(crate) fn to_value_map(
    params: &std::collections::BTreeMap<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (key, value) in params {
        out.insert(key.clone(), value.clone());
    }
    out
}
