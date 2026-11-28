//! Persistent interrupt-intent channel.
//!
//! A lock-outside handshake for suspending a workflow a `run` / `resume`
//! process is driving. The database has one writer at a time — the live
//! driver — so `todos interrupt` never opens one: it validates what it can
//! through a read-only store, publishes a request under
//! `<data_dir>/todos/.interrupts/<encoded-workflow-id>.json` and waits for
//! the driver to acknowledge it. The live driver polls that directory,
//! applies the request through the normal load → transition → CAS-commit
//! path on the writer it already holds, and rewrites the same file with its
//! decision. The file is the cross-process handshake; the database remains
//! the only source of workflow truth.
//!
//! The file has four states:
//!
//! * `requested` — written atomically by the publishing caller, bound to the
//!   generation it observed.
//! * `applied` — rewritten atomically by the driver after its commit (or
//!   when the request id is already in the event log). A waiting publisher
//!   may also confirm the durable Suspended state through a read-only store.
//! * `stale` — the workflow had left the request's generation before the
//!   driver looked (a later run is in progress); nothing was suspended. A
//!   request left behind by an earlier run can therefore never cancel a
//!   later one, and a publisher that is told so may retry against the fresh
//!   generation.
//! * `terminal` — the workflow is Completed/Failed; the request can never
//!   apply.
//!
//! Every request carries a unique `request_id`. The driver records it in the
//! `workflow_interrupted` event payload, which makes a crash between commit
//! and the applied rewrite idempotent: a `requested` file whose id is
//! already present in the event log is treated as applied, not re-applied.
//!
//! Acknowledgements stay on disk for [`ACK_RETENTION_MS`] so the publisher
//! can read them; the next poll after that removes them as leftovers.

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::types::*;

mod consume;
pub(crate) use consume::commit_interrupt;
pub use consume::consume_pending;

/// Directory (under the per-workdir data directory's `todos` root) holding
/// one intent file per workflow. Dot-prefixed so it cannot collide with the
/// debug-dump directories named after a workflow id.
pub const INTENT_DIR: &str = ".interrupts";

/// How long a driver leaves its acknowledgement on disk for the publisher
/// waiting for it — well above the publisher's bounded wait, so the ack is
/// never cleaned up under a waiting command; the next poll after that
/// removes it.
pub const ACK_RETENTION_MS: i64 = 60_000;

/// State of one intent file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// The interrupting command asked for a suspension.
    Requested,
    /// The driver committed the suspension (or found it already durable).
    Applied,
    /// The workflow had moved past the request's generation; nothing was
    /// suspended.
    Stale,
    /// The workflow is already Completed/Failed; nothing was suspended.
    Terminal,
}

/// The driver's decision on a request: which generation it decided on and
/// when. Suspended generation for `applied`, the generation that superseded
/// the request for `stale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub generation: u64,
    pub at_ms: i64,
}

/// One persistent interrupt request, later carrying the driver's decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptIntent {
    pub workflow_id: String,
    pub reason: String,
    pub request_id: String,
    pub requested_at_ms: i64,
    /// Generation the publisher observed. The driver applies the request
    /// only while the workflow is still at that generation, so a request
    /// left behind by an earlier run cannot suspend a later one.
    #[serde(default)]
    pub generation: u64,
    pub status: IntentStatus,
    /// Set by the driver together with every status it writes.
    #[serde(default)]
    pub decided: Option<Decision>,
}

/// Result of asking the driver-side consumer to look at the intent channel.
#[derive(Debug)]
pub enum ConsumeOutcome {
    /// No pending request: no file, or a leftover acknowledgement.
    None,
    /// A request was committed now; carries the suspended state.
    Applied(WorkflowState),
    /// The request was already durable (its id is in the event log); the file
    /// was marked applied and carries the current state.
    AlreadyApplied(WorkflowState),
    /// The workflow is past the request's generation (a later run); the
    /// request was marked stale and nothing was suspended.
    Stale(WorkflowState),
    /// The workflow is already Completed/Failed; the request was marked
    /// terminal and nothing was suspended.
    Terminal(WorkflowState),
}

/// Root directory of the intent channel for `workdir`.
pub fn root(workdir: &Path) -> PathBuf {
    opencoder_core::data_dir_for(workdir)
        .join("todos")
        .join(INTENT_DIR)
}

/// Intent file path for `workflow_id` under `root`.
pub fn path(root: &Path, workflow_id: &str) -> PathBuf {
    root.join(format!("{}.json", encode_workflow_id(workflow_id)))
}

/// Filename-safe, collision-free encoding of a workflow id. Safe bytes stay
/// readable; every other byte becomes `%XX`. Ids whose encoded form would be
/// too long for a filename use a stable digest instead.
pub fn encode_workflow_id(workflow_id: &str) -> String {
    let mut encoded = String::with_capacity(workflow_id.len());
    for byte in workflow_id.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    if encoded.len() > 180 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        workflow_id.hash(&mut hasher);
        return format!("long-{:x}", hasher.finish());
    }
    encoded
}

/// Atomically publish a new interrupt request for the workflow generation
/// the caller observed and return it.
pub async fn write_request(
    root: &Path,
    workflow_id: &str,
    reason: &str,
    generation: u64,
) -> Result<InterruptIntent> {
    let intent = InterruptIntent {
        workflow_id: workflow_id.to_string(),
        reason: reason.to_string(),
        request_id: format!("req-{}", ulid::Ulid::new()),
        requested_at_ms: opencoder_core::message::now_ms(),
        generation,
        status: IntentStatus::Requested,
        decided: None,
    };
    write(root, &intent).await?;
    Ok(intent)
}

/// Atomically write `intent` to its canonical path. Public so a caller can
/// restore a `requested` file when testing crash recovery; normal writers use
/// [`write_request`] so ids are never reused.
pub async fn write(root: &Path, intent: &InterruptIntent) -> Result<()> {
    let path = path(root, &intent.workflow_id);
    let parent = path.parent().context("intent path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = path.with_extension(format!("{}.tmp", intent.request_id));
    tokio::fs::write(&temporary, serde_json::to_vec_pretty(intent)?).await?;
    tokio::fs::rename(&temporary, &path).await?;
    Ok(())
}

/// Read the intent for `workflow_id`; `None` when no file exists.
pub async fn read(root: &Path, workflow_id: &str) -> Result<Option<InterruptIntent>> {
    let path = path(root, workflow_id);
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).with_context(|| {
            format!("decode interrupt intent {}", path.display())
        })?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("read interrupt intent {}", path.display()))
        }
    }
}

/// Rewrite the intent with the driver's `status` / `decision` when it still
/// carries `request_id` and is still `requested`. A newer request that
/// replaced the file is left untouched (`false`), so an acknowledgement can
/// never clobber a fresh request.
pub async fn mark(
    root: &Path,
    workflow_id: &str,
    request_id: &str,
    status: IntentStatus,
    decision: Decision,
) -> Result<bool> {
    let Some(mut intent) = read(root, workflow_id).await? else {
        return Ok(false);
    };
    if intent.request_id != request_id || intent.status != IntentStatus::Requested {
        return Ok(false);
    }
    intent.status = status;
    intent.decided = Some(decision);
    write(root, &intent).await?;
    Ok(true)
}

/// Whether an acknowledgement (`applied` / `stale` / `terminal`) is older
/// than [`ACK_RETENTION_MS`] and may be removed as a leftover. A file
/// without a decision is treated as expired.
pub fn expired(intent: &InterruptIntent, now_ms: i64) -> bool {
    match intent.decided {
        Some(decision) => now_ms.saturating_sub(decision.at_ms) >= ACK_RETENTION_MS,
        None => true,
    }
}

/// Remove the intent file; a missing file is already removed.
pub async fn remove(root: &Path, workflow_id: &str) -> Result<()> {
    let path = path(root, workflow_id);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove interrupt intent {}", path.display()))
        }
    }
}
