//! The one terminal transition of the topic state machine: write finished
//! metadata (reason, timestamp, optional final summary) to the share, then
//! flip every `(topic, node)` ledger row in the store to `finished`. Both
//! writes are idempotent, so a crash between them converges on retry.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use opencoder_core::message::now_ms;
use opencoder_store::Store;

use crate::fs_store;
use crate::types::{TopicMeta, TOPIC_FINISHED};

/// Terminal transition: write finished metadata, flip every ledger row.
pub(crate) async fn finish(
    store: &Arc<dyn Store>,
    team_root: &Path,
    meta: &mut TopicMeta,
    reason: &str,
    final_summary: Option<String>,
) -> Result<TopicMeta> {
    meta.status = TOPIC_FINISHED.to_string();
    meta.finish_reason = Some(reason.to_string());
    meta.finished_at = Some(now_ms());
    meta.final_summary = final_summary;
    fs_store::save_topic(team_root, meta)?;
    store
        .finish_team_topic_run(&meta.topic_id)
        .await
        .context("finish team topic runs")?;
    Ok(meta.clone())
}
