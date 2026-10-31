//! Data freshness plane: pure domain types and rules for policies, runs and reviews.
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Upper bound for parallel executors dispatched by a single policy.
pub const MAX_CONCURRENCY: u32 = 30;
/// Upper bound for entities captured by one run snapshot.
pub const MAX_RUN_ITEMS: usize = 500;
/// Upper bound for a candidate body kept with a run item.
pub const MAX_CANDIDATE_CHARS: usize = 8000;/// Freshness ladder for entity bodies; approval advances one rank at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", try_from = "String")]
pub enum DataLevel {
    #[default]
    Draft,
    Stable,
    Verified,
    Confident,
}

impl DataLevel {
    pub const ALL: [DataLevel; 4] = [Self::Draft, Self::Stable, Self::Verified, Self::Confident];

    pub fn parse(level: &str) -> Option<DataLevel> {
        Self::ALL.iter().copied().find(|item| item.as_str() == level)
    }

    pub fn rank(self) -> u8 {
        match self {
            Self::Draft => 0,
            Self::Stable => 1,
            Self::Verified => 2,
            Self::Confident => 3,
        }
    }

    pub fn next(self) -> Option<DataLevel> {
        Self::ALL.get(usize::from(self.rank() + 1)).copied()
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Stable => "stable",
            Self::Verified => "verified",
            Self::Confident => "confident",
        }
    }
}

impl TryFrom<String> for DataLevel {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or_else(|| format!("unknown data level: {value}"))
    }
}

impl std::fmt::Display for DataLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessExecutorKind {
    Agent,
    Dag,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessExecutor {
    pub kind: FreshnessExecutorKind,
    pub target: String,
    pub prompt_template: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FreshnessPolicy {
    pub id: Uuid,
    pub env_num: i64,
    pub entity_type_id: Uuid,
    pub name: String,
    pub description: String,
    pub executor: FreshnessExecutor,
    pub concurrency: u32,
    pub writeback_attribute_key: Option<String>,
    pub revision: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessItemStatus {
    #[default]
    Pending,
    Dispatched,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FreshnessRunItem {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub execution_id: Option<String>,
    pub status: FreshnessItemStatus,
    pub candidate: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessRunStats {
    pub total: u32,
    pub pending: u32,
    pub dispatched: u32,
    pub succeeded: u32,
    pub failed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessRunStatus {
    #[default]
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FreshnessRun {
    pub id: Uuid,
    pub env_num: i64,
    pub policy_id: Uuid,
    pub status: FreshnessRunStatus,
    pub items: Vec<FreshnessRunItem>,
    pub stats: FreshnessRunStats,
    pub revision: i64,
    pub is_deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessVerdict {
    Approve,
    Reject,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FreshnessReview {
    pub id: Uuid,
    pub env_num: i64,
    pub run_id: Uuid,
    pub policy_id: Uuid,
    pub entity_id: Uuid,
    pub from_level: DataLevel,
    pub to_level: DataLevel,
    pub verdict: FreshnessVerdict,
    pub comment: String,
    pub actor: String,
    pub created_at: String,
}

/// Validate policy fields shared by create and update paths.
pub fn validate_policy_fields(
    name: &str,
    entity_type_id: Uuid,
    executor: &FreshnessExecutor,
    concurrency: u32,
    writeback: Option<&str>,
) -> Result<(), AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("name is required"));
    }
    if trimmed.chars().count() > 200 {
        return Err(AppError::invalid("name exceeds 200 characters"));
    }
    if entity_type_id.is_nil() {
        return Err(AppError::invalid("entity_type_id is required"));
    }
    if executor.target.trim().is_empty() {
        return Err(AppError::invalid("executor target is required"));
    }
    if executor.kind == FreshnessExecutorKind::Agent && executor.prompt_template.trim().is_empty() {
        return Err(AppError::invalid("agent executors require a prompt template"));
    }
    if concurrency == 0 || concurrency > MAX_CONCURRENCY {
        return Err(AppError::invalid(format!(
            "concurrency must be between 1 and {MAX_CONCURRENCY}"
        )));
    }
    if writeback.is_some_and(|key| key.trim().is_empty()) {
        return Err(AppError::invalid(
            "writeback_attribute_key must be a non-empty key",
        ));
    }
    Ok(())
}

/// Recount item statuses; the server recomputes stats on every save.
pub fn recompute_stats(run: &FreshnessRun) -> FreshnessRunStats {
    let mut stats = FreshnessRunStats {
        total: run.items.len() as u32,
        ..FreshnessRunStats::default()
    };
    for item in &run.items {
        match item.status {
            FreshnessItemStatus::Pending => stats.pending += 1,
            FreshnessItemStatus::Dispatched => stats.dispatched += 1,
            FreshnessItemStatus::Succeeded => stats.succeeded += 1,
            FreshnessItemStatus::Failed => stats.failed += 1,
        }
    }
    stats
}

/// Legal status transitions: queued starts, running finishes, terminals are final.
pub fn run_status_allows_next(current: FreshnessRunStatus, next: FreshnessRunStatus) -> bool {
    use FreshnessRunStatus::*;
    matches!(
        (current, next),
        (Queued, Running) | (Running, Succeeded | Partial | Failed)
    )
}

/// Terminal status derived from the item mix; empty snapshots count as failed.
pub fn settle_status(run: &FreshnessRun) -> FreshnessRunStatus {
    let stats = recompute_stats(run);
    match (stats.total, stats.failed, stats.succeeded) {
        (0, _, _) => FreshnessRunStatus::Failed,
        (_, 0, _) => FreshnessRunStatus::Succeeded,
        (_, _, 0) => FreshnessRunStatus::Failed,
        _ => FreshnessRunStatus::Partial,
    }
}

/// Bound candidate bodies to the storage limit, measured in characters.
pub fn truncate_candidate(candidate: &str) -> String {
    candidate.chars().take(MAX_CANDIDATE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_levels_parse_order_and_advance() {
        for (index, level) in DataLevel::ALL.iter().enumerate() {
            assert_eq!(level.rank(), index as u8);
            assert_eq!(DataLevel::parse(level.as_str()), Some(*level));
        }
        for unknown in ["Stable", "unknown", ""] {
            assert_eq!(DataLevel::parse(unknown), None);
        }
        assert_eq!(DataLevel::Draft.next(), Some(DataLevel::Stable));
        assert_eq!(DataLevel::Stable.next(), Some(DataLevel::Verified));
        assert_eq!(DataLevel::Verified.next(), Some(DataLevel::Confident));
        assert_eq!(DataLevel::Confident.next(), None);
        assert_eq!(DataLevel::default(), DataLevel::Draft);
    }

    #[test]
    fn data_level_serde_uses_snake_case_names() {
        for level in DataLevel::ALL {
            let raw = serde_json::to_string(&level).unwrap();
            assert_eq!(raw, format!("\"{}\"", level.as_str()));
            assert_eq!(serde_json::from_str::<DataLevel>(&raw).unwrap(), level);
        }
        assert!(serde_json::from_str::<DataLevel>("\"Draft\"").is_err());
    }

    fn executor(kind: FreshnessExecutorKind, prompt: &str) -> FreshnessExecutor {
        FreshnessExecutor {
            kind,
            target: "executor-target".into(),
            prompt_template: prompt.into(),
        }
    }


    fn policy_error(name: &str, executor: &FreshnessExecutor, concurrency: u32) -> bool {
        validate_policy_fields(name, Uuid::new_v4(), executor, concurrency, None).is_err()
    }

    #[test]
    fn policy_validation_rejects_blank_names_and_bad_concurrency() {
        let agent = executor(FreshnessExecutorKind::Agent, "refresh {{name}}");
        assert!(validate_policy_fields("policy", Uuid::new_v4(), &agent, 1, None).is_ok());
        assert!(policy_error("  ", &agent, 1), "blank names are rejected");
        assert!(policy_error(&"x".repeat(201), &agent, 1));
        assert!(validate_policy_fields("p", Uuid::nil(), &agent, 1, None).is_err());
        for bad in [0u32, MAX_CONCURRENCY + 1] {
            assert!(policy_error("p", &agent, bad));
        }
        let valid = |concurrency| validate_policy_fields("p", Uuid::new_v4(), &agent, concurrency, Some("source"));
        assert!(valid(MAX_CONCURRENCY).is_ok());
        assert!(validate_policy_fields("p", Uuid::new_v4(), &agent, 1, Some(" ")).is_err());
    }

    #[test]
    fn executors_follow_kind_specific_prompt_rules() {
        let empty_prompt = executor(FreshnessExecutorKind::Agent, "");
        assert!(policy_error("policy", &empty_prompt, 1));
        let dag = executor(FreshnessExecutorKind::Dag, "");
        assert!(!policy_error("policy", &dag, 1));
        let missing_target = FreshnessExecutor {
            target: "  ".into(),
            ..dag
        };
        assert!(policy_error("policy", &missing_target, 1));
    }

    fn item(status: FreshnessItemStatus) -> FreshnessRunItem {
        FreshnessRunItem {
            status,
            ..FreshnessRunItem::default()
        }
    }

    fn run_of(status: FreshnessItemStatus, count: usize) -> FreshnessRun {
        FreshnessRun {
            items: vec![item(status); count],
            ..FreshnessRun::default()
        }
    }

    #[test]
    fn settle_status_covers_the_full_matrix() {
        use FreshnessRunStatus::*;
        assert_eq!(settle_status(&run_of(FreshnessItemStatus::Pending, 0)), Failed);
        assert_eq!(
            settle_status(&run_of(FreshnessItemStatus::Succeeded, 2)),
            Succeeded
        );
        assert_eq!(settle_status(&run_of(FreshnessItemStatus::Failed, 2)), Failed);
        let mut mixed = run_of(FreshnessItemStatus::Succeeded, 2);
        mixed.items.push(item(FreshnessItemStatus::Failed));
        assert_eq!(settle_status(&mixed), Partial);
    }

    #[test]
    fn stats_are_recounted_from_items() {
        let mut target = run_of(FreshnessItemStatus::Pending, 1);
        target.items.push(item(FreshnessItemStatus::Dispatched));
        target.items.push(item(FreshnessItemStatus::Succeeded));
        target.items.push(item(FreshnessItemStatus::Succeeded));
        target.items.push(item(FreshnessItemStatus::Failed));
        assert_eq!(
            recompute_stats(&target),
            FreshnessRunStats {
                total: 5,
                pending: 1,
                dispatched: 1,
                succeeded: 2,
                failed: 1
            }
        );
        assert_eq!(recompute_stats(&run_of(FreshnessItemStatus::Pending, 0)).total, 0);
    }

    #[test]
    fn run_status_transitions_never_skip_states() {
        use FreshnessRunStatus::*;
        assert!(run_status_allows_next(Queued, Running));
        assert!(!run_status_allows_next(Queued, Queued));
        for skipped in [Succeeded, Partial, Failed] {
            assert!(!run_status_allows_next(Queued, skipped));
        }
        for finish in [Succeeded, Partial, Failed] {
            assert!(run_status_allows_next(Running, finish));
        }
        assert!(!run_status_allows_next(Running, Running));
        for terminal in [Succeeded, Partial, Failed] {
            for next in [Queued, Running, Succeeded, Partial, Failed] {
                assert!(!run_status_allows_next(terminal, next));
            }
        }
    }

    #[test]
    fn candidates_are_truncated_by_characters() {
        assert_eq!(truncate_candidate("abc"), "abc");
        let long: String = "ab".repeat(MAX_CANDIDATE_CHARS + 5);
        assert_eq!(truncate_candidate(&long).chars().count(), MAX_CANDIDATE_CHARS);
    }
}
