use serde::{Deserialize, Serialize};

/// Status of a `(topic, node)` run row while the node is still working.
pub const TEAM_RUN_EXECUTING: &str = "executing";

/// Status of a `(topic, node)` run row once its part of the topic is done.
pub const TEAM_RUN_FINISHED: &str = "finished";

/// One row of the `team_topic_runs` ledger: which node is (or was) working
/// which team topic. The opencoder-team runtime fans a topic out to N nodes;
/// this table is the durable pairing record — `status` starts `executing`
/// and flips to `finished` (per-row upsert or topic-wide `finish`), while
/// `created_at` is frozen at first insert so a refresh never restarts the
/// run's clock. Pure data: the team runtime lives above the Store, which
/// only persists it (see `libsql_store/team_runs.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamTopicRunRecord {
    /// ULID of the team topic (one fan-out unit).
    pub topic_id: String,
    /// `nodes.id` of the worker this row is paired with (FK, cascade).
    pub node_id: String,
    /// [`TEAM_RUN_EXECUTING`] | [`TEAM_RUN_FINISHED`].
    pub status: String,
    /// Unix-ms timestamp of the row's first insert (stable across upserts).
    pub created_at: i64,
}
