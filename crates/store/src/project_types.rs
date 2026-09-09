//! Project-module domain types (goals / milestones / todos / runs).
//!
//! Status and kind enums serialize as snake_case strings so the JSON wire form
//! matches the DB columns byte-for-byte (no mapping layer needed). `parse`
//! returns `Option` so an unknown string from a future version surfaces as a
//! caller-visible error instead of silently coercing to a default state.

use serde::{Deserialize, Serialize};

/// Lifecycle of a project goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectGoalStatus {
    Active,
    Archived,
}

impl ProjectGoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectGoalStatus::Active => "active",
            ProjectGoalStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(ProjectGoalStatus::Active),
            "archived" => Some(ProjectGoalStatus::Archived),
            _ => None,
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ProjectGoalStatus::Archived)
    }
}

/// Lifecycle of a milestone within a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMilestoneStatus {
    Planned,
    InProgress,
    Done,
}

impl ProjectMilestoneStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectMilestoneStatus::Planned => "planned",
            ProjectMilestoneStatus::InProgress => "in_progress",
            ProjectMilestoneStatus::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "planned" => Some(ProjectMilestoneStatus::Planned),
            "in_progress" => Some(ProjectMilestoneStatus::InProgress),
            "done" => Some(ProjectMilestoneStatus::Done),
            _ => None,
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ProjectMilestoneStatus::Done)
    }
}

/// Lifecycle of a project todo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTodoStatus {
    Draft,
    Planned,
    Running,
    Done,
    Failed,
}

impl ProjectTodoStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectTodoStatus::Draft => "draft",
            ProjectTodoStatus::Planned => "planned",
            ProjectTodoStatus::Running => "running",
            ProjectTodoStatus::Done => "done",
            ProjectTodoStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(ProjectTodoStatus::Draft),
            "planned" => Some(ProjectTodoStatus::Planned),
            "running" => Some(ProjectTodoStatus::Running),
            "done" => Some(ProjectTodoStatus::Done),
            "failed" => Some(ProjectTodoStatus::Failed),
            _ => None,
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ProjectTodoStatus::Done | ProjectTodoStatus::Failed)
    }
}

/// What a todo run executed: planning pass or execution pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTodoRunKind {
    Plan,
    Execute,
}

impl ProjectTodoRunKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectTodoRunKind::Plan => "plan",
            ProjectTodoRunKind::Execute => "execute",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(ProjectTodoRunKind::Plan),
            "execute" => Some(ProjectTodoRunKind::Execute),
            _ => None,
        }
    }
}

/// Lifecycle of a todo run (one plan or execute attempt).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectTodoRunStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl ProjectTodoRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectTodoRunStatus::Running => "running",
            ProjectTodoRunStatus::Done => "done",
            ProjectTodoRunStatus::Failed => "failed",
            ProjectTodoRunStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "running" => Some(ProjectTodoRunStatus::Running),
            "done" => Some(ProjectTodoRunStatus::Done),
            "failed" => Some(ProjectTodoRunStatus::Failed),
            "cancelled" => Some(ProjectTodoRunStatus::Cancelled),
            _ => None,
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ProjectTodoRunStatus::Done
                | ProjectTodoRunStatus::Failed
                | ProjectTodoRunStatus::Cancelled
        )
    }
}

/// A long-lived project goal (`project_goals` row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGoalRecord {
    pub id: String,
    pub title: String,
    pub detail_md: Option<String>,
    pub status: ProjectGoalStatus,
    pub sort: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Partial update for [`ProjectGoalRecord`]; `None` fields stay unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectGoalPatch {
    pub title: Option<String>,
    pub detail_md: Option<String>,
    pub status: Option<ProjectGoalStatus>,
    pub sort: Option<i64>,
}

/// A standalone initiative, optionally belonging to a goal (`project_milestones` row).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMilestoneRecord {
    pub id: String,
    pub goal_id: Option<String>,
    pub title: String,
    pub detail_md: Option<String>,
    pub status: ProjectMilestoneStatus,
    pub sort: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Partial update for [`ProjectMilestoneRecord`]; `None` fields stay
/// unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectMilestonePatch {
    pub goal_id: Option<Option<String>>,
    pub title: Option<String>,
    pub detail_md: Option<String>,
    pub status: Option<ProjectMilestoneStatus>,
    pub sort: Option<i64>,
}

/// Which executor drives a todo: the single built-in agent flow, a named
/// team definition, an inline DAG spec, or a brain-routed capability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectExecutorKind {
    #[default]
    Agent,
    Team,
    Dag,
    Brain,
}

impl ProjectExecutorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectExecutorKind::Agent => "agent",
            ProjectExecutorKind::Team => "team",
            ProjectExecutorKind::Dag => "dag",
            ProjectExecutorKind::Brain => "brain",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(ProjectExecutorKind::Agent),
            "team" => Some(ProjectExecutorKind::Team),
            "dag" => Some(ProjectExecutorKind::Dag),
            "brain" => Some(ProjectExecutorKind::Brain),
            _ => None,
        }
    }
}

/// A project todo (`project_todos` row). `milestone_id == None` is the
/// milestone-less backlog. The executor dimension says WHO drives the todo:
/// `agent` keeps the single-agent flow (`agent` names it), while
/// team/dag/brain carry the target in `executor_ref` (team/dag name or
/// pinned brain capability id) and/or the inline JSON spec in
/// `executor_spec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTodoRecord {
    pub id: String,
    pub milestone_id: Option<String>,
    pub title: String,
    pub draft: String,
    pub plan_md: Option<String>,
    pub status: ProjectTodoStatus,
    pub agent: String,
    #[serde(default)]
    pub executor_kind: ProjectExecutorKind,
    #[serde(default)]
    pub executor_ref: Option<String>,
    #[serde(default)]
    pub executor_spec: Option<String>,
    pub active_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Partial update for [`ProjectTodoRecord`].
///
/// `Option<Option<T>>` semantics: outer `None` = leave unchanged,
/// `Some(None)` = clear to NULL, `Some(Some(v))` = set to `v`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectTodoPatch {
    pub title: Option<String>,
    pub draft: Option<String>,
    pub plan_md: Option<Option<String>>,
    pub status: Option<ProjectTodoStatus>,
    pub agent: Option<String>,
    pub executor_kind: Option<ProjectExecutorKind>,
    pub executor_ref: Option<Option<String>>,
    pub executor_spec: Option<Option<String>>,
    pub milestone_id: Option<Option<String>>,
    pub active_session_id: Option<Option<String>>,
}

/// One plan/execute attempt against a todo (`project_todo_runs` row);
/// `version` numbers the attempts per todo. The executor dimension mirrors
/// the todo's own: brain-routed runs additionally record their provenance
/// (`capability_id` / `plan_id`), dag/team runs point at their artifact
/// root / topic id via `output_ref`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTodoRunRecord {
    #[serde(default)]
    pub input_snapshot: Option<String>,
    #[serde(default)]
    pub trace_manifest: Option<String>,
    pub id: String,
    pub todo_id: String,
    pub kind: ProjectTodoRunKind,
    pub version: i64,
    pub plan_md: Option<String>,
    pub output_md: Option<String>,
    pub agent: String,
    #[serde(default)]
    pub executor_kind: ProjectExecutorKind,
    #[serde(default)]
    pub capability_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub output_ref: Option<String>,
    pub session_id: Option<String>,
    pub status: ProjectTodoRunStatus,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ProjectRunText {
    Text(String),
    Omitted {
        omitted: bool,
        total_bytes: u64,
        read_via: &'static str,
        field: String,
    },
}

/// Inspect projection of one project todo with bounded text columns.
/// `executor_spec` is deliberately excluded — inline specs can be huge, the
/// bounded views only carry the kind + ref.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectTodoSummary {
    pub id: String,
    pub milestone_id: Option<String>,
    pub title: String,
    pub draft: ProjectRunText,
    pub plan_md: Option<ProjectRunText>,
    pub status: ProjectTodoStatus,
    pub agent: String,
    pub executor_kind: ProjectExecutorKind,
    pub executor_ref: Option<String>,
    pub active_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectTodoRunSummary {
    pub input_snapshot: Option<ProjectRunText>,
    pub trace_manifest: Option<ProjectRunText>,
    pub id: String,
    pub todo_id: String,
    pub kind: ProjectTodoRunKind,
    pub version: i64,
    pub plan_md: Option<ProjectRunText>,
    pub output_md: Option<ProjectRunText>,
    pub agent: String,
    pub executor_kind: ProjectExecutorKind,
    pub capability_id: Option<String>,
    pub plan_id: Option<String>,
    pub output_ref: Option<String>,
    pub session_id: Option<String>,
    pub status: ProjectTodoRunStatus,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectTodoRunPage {
    pub runs: Vec<ProjectTodoRunSummary>,
    pub next_version: Option<i64>,
}

/// Partial update for [`ProjectTodoRunRecord`]; `None` fields stay unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectTodoRunPatch {
    pub input_snapshot: Option<String>,
    pub trace_manifest: Option<String>,
    pub plan_md: Option<String>,
    pub output_md: Option<String>,
    pub output_ref: Option<String>,
    pub capability_id: Option<String>,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub status: Option<ProjectTodoRunStatus>,
    pub finished_at: Option<i64>,
}

/// Bound the combined page, including fields individually below the chunk limit.
pub(crate) fn project_run_page(
    mut runs: Vec<ProjectTodoRunSummary>,
    limit: usize,
) -> anyhow::Result<ProjectTodoRunPage> {
    let mut bytes = 0usize;
    let mut count = 0;
    for run in runs.iter().take(limit) {
        let size = serde_json::to_vec(run)?.len();
        if bytes + size > 512 * 1024 && count > 0 {
            break;
        }
        bytes += size;
        count += 1;
    }
    let more = runs.len() > count;
    runs.truncate(count);
    Ok(ProjectTodoRunPage {
        next_version: more.then(|| runs.last().unwrap().version),
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_as_str_round_trip_every_variant() {
        let goals = [ProjectGoalStatus::Active, ProjectGoalStatus::Archived];
        for v in goals {
            assert_eq!(ProjectGoalStatus::parse(v.as_str()), Some(v));
        }
        for v in [
            ProjectMilestoneStatus::Planned,
            ProjectMilestoneStatus::InProgress,
            ProjectMilestoneStatus::Done,
        ] {
            assert_eq!(ProjectMilestoneStatus::parse(v.as_str()), Some(v));
        }
        for v in [
            ProjectTodoStatus::Draft,
            ProjectTodoStatus::Planned,
            ProjectTodoStatus::Running,
            ProjectTodoStatus::Done,
            ProjectTodoStatus::Failed,
        ] {
            assert_eq!(ProjectTodoStatus::parse(v.as_str()), Some(v));
        }
        for v in [ProjectTodoRunKind::Plan, ProjectTodoRunKind::Execute] {
            assert_eq!(ProjectTodoRunKind::parse(v.as_str()), Some(v));
        }
        for v in [
            ProjectTodoRunStatus::Running,
            ProjectTodoRunStatus::Done,
            ProjectTodoRunStatus::Failed,
            ProjectTodoRunStatus::Cancelled,
        ] {
            assert_eq!(ProjectTodoRunStatus::parse(v.as_str()), Some(v));
        }
        for v in [
            ProjectExecutorKind::Agent,
            ProjectExecutorKind::Team,
            ProjectExecutorKind::Dag,
            ProjectExecutorKind::Brain,
        ] {
            assert_eq!(ProjectExecutorKind::parse(v.as_str()), Some(v));
        }
    }

    #[test]
    fn unknown_string_parses_to_none() {
        // Forward-compat: a status string written by a newer version must
        // surface as None, never silently coerce.
        assert_eq!(ProjectGoalStatus::parse("deleted"), None);
        assert_eq!(ProjectMilestoneStatus::parse("active"), None);
        assert_eq!(ProjectTodoStatus::parse("in_progress"), None);
        assert_eq!(ProjectTodoRunKind::parse("review"), None);
        assert_eq!(ProjectTodoRunStatus::parse("paused"), None);
        assert_eq!(ProjectTodoStatus::parse(""), None);
        assert_eq!(ProjectExecutorKind::parse("workflow"), None);
        assert_eq!(ProjectExecutorKind::parse("Agent"), None);
    }

    #[test]
    fn serde_snake_case_round_trips_match_db_strings() {
        // JSON wire form must equal the DB column strings exactly.
        assert_eq!(
            serde_json::to_string(&ProjectMilestoneStatus::InProgress).unwrap(),
            "\"in_progress\""
        );
        assert_eq!(
            serde_json::to_string(&ProjectTodoStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&ProjectTodoRunStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
        assert_eq!(
            serde_json::to_string(&ProjectExecutorKind::Team).unwrap(),
            "\"team\""
        );
        let back: ProjectMilestoneStatus = serde_json::from_str("\"in_progress\"").unwrap();
        assert_eq!(back, ProjectMilestoneStatus::InProgress);
        let back: ProjectExecutorKind = serde_json::from_str("\"dag\"").unwrap();
        assert_eq!(back, ProjectExecutorKind::Dag);
    }

    #[test]
    fn executor_fields_default_on_legacy_json() {
        // Rows written before the executor dimension existed deserialize to
        // the agent flow with all optional refs absent.
        let todo: ProjectTodoRecord = serde_json::from_str(
            "{\"id\":\"t\",\"milestone_id\":null,\"title\":\"t\",\"draft\":\"d\",\
             \"plan_md\":null,\"status\":\"draft\",\"agent\":\"act\",\
             \"active_session_id\":null,\"created_at\":1,\"updated_at\":1}",
        )
        .unwrap();
        assert_eq!(todo.executor_kind, ProjectExecutorKind::Agent);
        assert_eq!(todo.executor_ref, None);
        assert_eq!(todo.executor_spec, None);
        assert_eq!(ProjectExecutorKind::default(), ProjectExecutorKind::Agent);

        let run: ProjectTodoRunRecord = serde_json::from_str(
            "{\"id\":\"r\",\"todo_id\":\"t\",\"kind\":\"execute\",\"version\":1,\
             \"plan_md\":null,\"output_md\":null,\"agent\":\"act\",\"session_id\":null,\
             \"status\":\"running\",\"started_at\":1,\"finished_at\":null,\"created_at\":1}",
        )
        .unwrap();
        assert_eq!(run.executor_kind, ProjectExecutorKind::Agent);
        assert_eq!(run.capability_id, None);
        assert_eq!(run.plan_id, None);
        assert_eq!(run.output_ref, None);
    }

    #[test]
    fn terminal_states() {
        assert!(!ProjectGoalStatus::Active.is_terminal());
        assert!(ProjectGoalStatus::Archived.is_terminal());
        assert!(!ProjectMilestoneStatus::InProgress.is_terminal());
        assert!(ProjectMilestoneStatus::Done.is_terminal());
        assert!(!ProjectTodoStatus::Running.is_terminal());
        assert!(ProjectTodoStatus::Done.is_terminal());
        assert!(ProjectTodoStatus::Failed.is_terminal());
        assert!(!ProjectTodoRunStatus::Running.is_terminal());
        assert!(ProjectTodoRunStatus::Done.is_terminal());
        assert!(ProjectTodoRunStatus::Failed.is_terminal());
        assert!(ProjectTodoRunStatus::Cancelled.is_terminal());
    }
}
