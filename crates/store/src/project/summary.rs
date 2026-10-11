//! A user/Operator-authored overview, separate from native execution results.
use serde::{Deserialize, Serialize};

pub const SUMMARY_HISTORY_LIMIT: i64 = 10;

#[derive(Debug, Clone, Copy)]
pub enum SummaryScope {
    Project,
    Initiative,
}

impl SummaryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Initiative => "initiative",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryVersion {
    pub version: i64,
    pub current_summary: String,
    pub created_at: i64,
    pub restored_from: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSummary {
    #[serde(default)]
    pub current_summary: String,
    pub summary_updated_at: Option<i64>,
}

impl ProjectSummary {
    pub fn new(text: Option<String>, now: i64) -> Self {
        Self {
            current_summary: text.as_deref().unwrap_or_default().trim().to_owned(),
            summary_updated_at: text.map(|_| now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_summary_writes_have_a_timestamp() {
        assert_eq!(ProjectSummary::new(None, 12).summary_updated_at, None);
        let summary = ProjectSummary::new(Some(" progress ".into()), 12);
        assert_eq!(summary.current_summary, "progress");
        assert_eq!(summary.summary_updated_at, Some(12));
        assert_eq!(
            ProjectSummary::new(Some(" ".into()), 13).current_summary,
            ""
        );
    }
}
