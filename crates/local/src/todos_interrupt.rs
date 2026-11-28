//! `todos interrupt` dispatch: the lock-outside request path.
//!
//! The per-workdir database has one writer at a time — the `run` / `resume`
//! process driving the workflow — and this command is not it. It never opens
//! a writer (no `open_store`, no writable connection): whatever it needs from
//! the database it reads through `TursoStore::open_reader`, and the
//! suspension itself is committed by the live driver on the writer it already
//! holds. The handshake is the persistent intent channel
//! (`opencoder_todos::interrupt_intent`):
//!
//! 1. Open the store read-only; no database means no workflow to interrupt,
//!    and the command never creates one. Load the workflow and refuse what no
//!    driver could ever suspend (Completed / Failed) or need not (anything
//!    that is not Running), before anything is published.
//! 2. Publish a unique request (`request_id`, bound to the observed
//!    generation) under `<data_dir>/todos/.interrupts/<id>.json`. The file is
//!    durable: the request outlives this process and is applied by the next
//!    driver even when none is alive now.
//! 3. Wait a bounded time for the driver's acknowledgement in the same file
//!    (`applied` / `stale` / `terminal`), confirming `applied` through the
//!    reader as well. `stale` — the driver had moved the generation before it
//!    looked — is retried with a fresh request against the new generation.
//! 4. Print the suspended line. A timeout is an actionable error (the request
//!    stays queued and the next `todos resume <id>` parks the workflow before
//!    scheduling anything), never a silent success.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use opencoder_store::Store;
use opencoder_todos::{
    interrupt_intent::{self, IntentStatus, InterruptIntent},
    persistence,
    types::WorkflowStatus,
    WorkflowState,
};

/// Reason recorded as the workflow's `terminal_reason` and in the
/// `workflow_interrupted` event payload.
const REASON: &str = "CLI interrupt requested";

/// Upper bound on waiting for a live driver to apply one request. A live
/// driver polls the channel every 250ms; the bound only matters when no
/// driver is alive, and then it is what turns a queued request into an
/// actionable error instead of a hang.
pub(crate) const APPLY_DEADLINE: Duration = Duration::from_secs(15);

/// Cadence of the acknowledgement poll (intent file, then the read-only store).
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Requests the command publishes before it gives up: the first one plus one
/// fresh request per `stale` acknowledgement.
const MAX_ATTEMPTS: u32 = 3;

/// Dispatch `todos interrupt <id>`: queue the suspension for the live driver
/// and report the state it committed.
pub async fn dispatch(workdir: &Path, workflow_id: &str) -> Result<()> {
    let state = dispatch_with(workdir, workflow_id, APPLY_DEADLINE).await?;
    println!("{}", suspended_line(&state.workflow_id, state.generation));
    Ok(())
}

/// [`dispatch`] with an explicit wait bound, so tests can exercise the
/// no-driver path without sleeping the production deadline. Returns the
/// suspended state the driver committed.
pub(crate) async fn dispatch_with(
    workdir: &Path,
    workflow_id: &str,
    wait: Duration,
) -> Result<WorkflowState> {
    let Some(reader) = crate::session_cmd::open_reader_store(workdir).await? else {
        anyhow::bail!("todo workflow not found: {workflow_id}");
    };
    let store: Arc<dyn Store> = Arc::new(reader.clone());
    let database = crate::session_cmd::database_for(workdir);
    // A reader never migrates: a read that fails on a store older than this
    // build is reported with the remedy, not the engine's raw query error.
    let (_, state) =
        crate::session_cmd::inspect(&reader, &database, load(&store, workflow_id)).await?;
    let intent_root = interrupt_intent::root(workdir);
    let deadline = Instant::now() + wait;
    let mut attempts_left = MAX_ATTEMPTS;
    let mut state = state;
    loop {
        if !needs_request(&state)? {
            return Ok(state);
        }
        attempts_left -= 1;
        let request =
            interrupt_intent::write_request(&intent_root, workflow_id, REASON, state.generation)
                .await?;
        match await_decision(&store, &intent_root, &request, deadline).await? {
            Decision::Suspended(latest) => return Ok(latest),
            Decision::Stale(latest) if attempts_left > 0 => state = latest,
            Decision::Stale(latest) => anyhow::bail!(
                "todo workflow {workflow_id} was not interrupted: it kept advancing while the \
                 request was pending (now at generation {}); nothing was suspended. Retry the \
                 interrupt",
                latest.generation
            ),
            Decision::TimedOut => anyhow::bail!(
                "todo workflow {workflow_id} was not interrupted: no `todos run` or `todos resume` \
                 process applied the request within {}s. The request stays queued at {} and the \
                 next `opencoder todos resume {workflow_id}` parks the workflow before scheduling \
                 anything; delete that file to discard it",
                wait.as_secs(),
                interrupt_intent::path(&intent_root, workflow_id).display()
            ),
        }
    }
}

/// What the command does with the state it reads: `Ok(true)` publishes a
/// request against it; `Ok(false)` when the workflow is already parked
/// (nothing left to stop, the suspended state is reported as-is); an error
/// names what no driver could ever suspend or need not.
fn needs_request(state: &WorkflowState) -> Result<bool> {
    if matches!(
        state.status,
        WorkflowStatus::Completed | WorkflowStatus::Failed
    ) {
        anyhow::bail!("cannot interrupt terminal workflow {}", state.workflow_id);
    }
    match state.status {
        WorkflowStatus::Running => Ok(true),
        WorkflowStatus::Suspended => Ok(false),
        status => anyhow::bail!(
            "todo workflow {} is not running (status: {}); nothing to interrupt",
            state.workflow_id,
            serde_json::to_value(status).unwrap_or_default()
        ),
    }
}

enum Decision {
    /// The suspension is durable (acknowledged, or confirmed through the reader).
    Suspended(WorkflowState),
    /// The driver found the workflow past the request's generation.
    Stale(WorkflowState),
    /// Nobody applied the request before the deadline.
    TimedOut,
}

/// Poll the intent file and the read-only store until the driver decides on
/// `request` or `deadline` passes.
async fn await_decision(
    store: &Arc<dyn Store>,
    root: &Path,
    request: &InterruptIntent,
    deadline: Instant,
) -> Result<Decision> {
    loop {
        let mine = interrupt_intent::read(root, &request.workflow_id)
            .await?
            .filter(|intent| intent.request_id == request.request_id);
        match mine.map(|intent| intent.status) {
            Some(IntentStatus::Applied) => {
                return Ok(Decision::Suspended(latest(store, request).await?))
            }
            Some(IntentStatus::Stale) => return Ok(Decision::Stale(latest(store, request).await?)),
            Some(IntentStatus::Terminal) => {
                anyhow::bail!("cannot interrupt terminal workflow {}", request.workflow_id)
            }
            // Still requested, or replaced by another command's request:
            // the durable state decides below.
            _ => {}
        }
        let state = latest(store, request).await?;
        match state.status {
            WorkflowStatus::Suspended => return Ok(Decision::Suspended(state)),
            WorkflowStatus::Completed | WorkflowStatus::Failed => {
                anyhow::bail!("cannot interrupt terminal workflow {}", request.workflow_id)
            }
            WorkflowStatus::Pending | WorkflowStatus::Running => {}
        }
        if Instant::now() >= deadline {
            return Ok(Decision::TimedOut);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn latest(store: &Arc<dyn Store>, request: &InterruptIntent) -> Result<WorkflowState> {
    Ok(load(store, &request.workflow_id).await?.1)
}

async fn load(
    store: &Arc<dyn Store>,
    workflow_id: &str,
) -> Result<(opencoder_todos::WorkflowSpec, WorkflowState)> {
    persistence::load(store, workflow_id)
        .await?
        .with_context(|| format!("todo workflow not found: {workflow_id}"))
}

/// Human line for a durable suspension. Kept pure so the output contract is
/// testable without capturing stdout.
pub(crate) fn suspended_line(workflow_id: &str, generation: u64) -> String {
    format!("{workflow_id} suspended at generation {generation}")
}

#[cfg(test)]
mod tests;
