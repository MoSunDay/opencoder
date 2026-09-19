//! `schedules.json` domain config: cron-scheduled control-plane jobs
//! (`schedules`), the fifth hard-cut domain file next to
//! mcp.json / cli.json / skills.json / ap.json.

mod validate;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fleet::ExecutionKind;
use crate::schedule::{render_params, CronExpr};
use validate::{base_instant, to_value_map, validate_brain_params, validate_target};

pub use validate::validate_id;

/// Longest schedule id. Deterministic execution ids embed it as
/// `<kind prefix>-<schedule id>-<13-digit fire ms>`, and execution ids are
/// capped at 64 chars by `fleet::valid_id` — 40 keeps every kind within it.
pub const MAX_SCHEDULE_ID_LEN: usize = 40;

/// The five triggerable target kinds. `brain` routes through the brain-run
/// entry point; the rest submit fleet executions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    Brain,
    Team,
    Todos,
    Agent,
    Dag,
}

impl ScheduleKind {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "brain" => Self::Brain,
            "team" => Self::Team,
            "todos" => Self::Todos,
            "agent" => Self::Agent,
            "dag" => Self::Dag,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brain => "brain",
            Self::Team => "team",
            Self::Todos => "todos",
            Self::Agent => "agent",
            Self::Dag => "dag",
        }
    }
    /// The execution kind the scheduler submits to (brain goes through the
    /// brain-run entry point instead, but still records this kind).
    pub fn execution_kind(self) -> ExecutionKind {
        match self {
            Self::Brain => ExecutionKind::Brain,
            Self::Team => ExecutionKind::Team,
            Self::Todos => ExecutionKind::Todos,
            Self::Agent => ExecutionKind::Agent,
            Self::Dag => ExecutionKind::Dag,
        }
    }
}

/// What to do when the previous fired execution is still running: `skip`
/// waits for a later tick, `allow` stacks a new execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleOverlap {
    #[default]
    Skip,
    Allow,
}

impl ScheduleOverlap {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "skip" => Self::Skip,
            "allow" => Self::Allow,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Allow => "allow",
        }
    }
}

/// One declared cronjob: target object + cron expression + params (string
/// values may carry `{{now...}}` time templates resolved at fire time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleJob {
    /// `[A-Za-z0-9_-]`, starts alphanumeric, 1..=`MAX_SCHEDULE_ID_LEN`.
    pub id: String,
    /// 5-field cron (`分 时 日 月 周`), optional leading seconds field.
    pub cron: String,
    /// Fixed offset (`UTC`, `+08:00`); defaults to UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub kind: ScheduleKind,
    /// agent/team: target name; todos: `template/version`; dag: definition id;
    /// brain: plan-def id.
    pub target: String,
    /// Passed to the target as `CreateExecution.input` (agent/team/todos).
    /// brain reads `objective`/`inputs`/`plan{,version}`/`mode` from here.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub overlap: ScheduleOverlap,
    /// Optional node pin, forwarded to the execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl ScheduleJob {
    /// Full structural validation: id charset, cron parse, timezone parse,
    /// kind/target contract, params render dry-run (at an arbitrary instant —
    /// the templates must not depend on "when").
    pub fn validate(&self) -> Result<(), String> {
        validate_id(&self.id)?;
        if self.enabled && self.cron.trim().is_empty() {
            return Err(format!("schedule {}: cron must not be empty", self.id));
        }
        if self.target.trim().is_empty() {
            return Err(format!("schedule {}: target must not be empty", self.id));
        }
        validate_target(self)?;
        if self.kind == ScheduleKind::Dag && !self.params.is_empty() {
            return Err(format!(
                "schedule {}: dag targets take no params (node pinning is the `node_id` field)",
                self.id
            ));
        }
        if self
            .node_id
            .as_deref()
            .is_some_and(|node| !crate::fleet::valid_id(node))
        {
            return Err(format!("schedule {}: invalid node_id", self.id));
        }
        if self.enabled {
            // Disabled schedules are inert: skip cron/params checks so a job
            // can be parked with a half-edited expression.
            CronExpr::parse(&self.cron, self.timezone.as_deref())
                .map_err(|error| format!("schedule {}: {error}", self.id))?;
            // Dry-run the time templates at a fixed instant: any token error
            // must surface at config time, not at 3am.
            render_params(&Value::Object(to_value_map(&self.params)), base_instant())
                .map_err(|error| format!("schedule {}: {error}", self.id))?;
            // brain params mirror the fire-time contract so a bad objective /
            // mode fails here instead of landing as an `error` ledger row.
            if self.kind == ScheduleKind::Brain {
                validate_brain_params(self)?;
            }
        }
        Ok(())
    }
}

/// `schedules.json` body. `{}` == `Default` (no schedules).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SchedulesConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<ScheduleJob>,
    /// Control-plane scan cadence in seconds (default 15; minimum 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_interval_secs: Option<u64>,
}

impl SchedulesConfig {
    pub fn is_empty(&self) -> bool {
        self.schedules.is_empty() && self.scan_interval_secs.is_none()
    }

    pub fn job(&self, id: &str) -> Option<&ScheduleJob> {
        self.schedules.iter().find(|job| job.id == id)
    }

    /// Lenient parse used by the control-plane scheduler: structurally valid
    /// JSON with invalid entries warns and skips (fail-soft — one bad job
    /// must never take the control plane down). Strict validation is what
    /// `ScheduleJob::validate` is for.
    pub fn from_value(value: &Value) -> Self {
        let mut config = Self::default();
        let Some(entries) = value.as_object() else {
            return config;
        };
        if let Some(raw) = entries.get("schedules") {
            match serde_json::from_value::<Vec<ScheduleJob>>(raw.clone()) {
                Ok(jobs) => config.schedules = jobs,
                Err(error) => {
                    tracing::warn!(
                        "schedules.json has an invalid `schedules` array: {error}; ignoring"
                    );
                }
            }
        }
        config.scan_interval_secs = entries
            .get("scan_interval_secs")
            .and_then(parse_scan_interval_secs);
        config
    }
}

/// Shared `scan_interval_secs` handling for `from_value` and the legacy
/// config.json merge: must be a u64 >= 1; zero is rejected with the same
/// warning on both paths (P3: one behavior, not two).
fn parse_scan_interval_secs(raw: &Value) -> Option<u64> {
    raw.as_u64().filter(|secs| {
        if *secs == 0 {
            tracing::warn!("schedules scan_interval_secs must be >= 1; using default");
        }
        *secs > 0
    })
}

/// Domain-file routing: effective `schedules.json` (project first, else
/// global home), when one exists.
pub fn schedules_path(working_dir: &Path) -> Option<PathBuf> {
    super::domain::effective_path(working_dir, "schedules")
}

/// Fail-soft load for the control-plane scheduler: missing file -> default,
/// corrupt entries skipped with a warning.
pub fn load_schedules(working_dir: &Path) -> SchedulesConfig {
    match super::domain::read_effective(working_dir, "schedules") {
        Some(value) => SchedulesConfig::from_value(&value),
        None => SchedulesConfig::default(),
    }
}

/// Domain merge: `schedules.json`'s body IS the config (autopilot-style
/// whole-object merge; the array replaces wholesale).
pub(crate) fn merge(cfg: &mut SchedulesConfig, entries: &serde_json::Map<String, Value>) {
    if let Some(raw) = entries.get("schedules") {
        match serde_json::from_value::<Vec<ScheduleJob>>(raw.clone()) {
            Ok(jobs) => cfg.schedules = jobs,
            Err(error) => {
                tracing::warn!(
                    "schedules.json carries an invalid schedules array: {error}; ignoring"
                )
            }
        }
    }
    if let Some(raw) = entries.get("scan_interval_secs") {
        cfg.scan_interval_secs = parse_scan_interval_secs(raw);
    }
}

#[cfg(test)]
mod tests;
