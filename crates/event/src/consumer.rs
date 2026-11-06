//! Durable event consumer pump: replay the event log behind a cursor.
//!
//! [`run_pump`] is generic infrastructure: it reads the events strictly
//! after a consumer's persisted cursor, dispatches each one through the
//! explicit [`DispatchDecision`] verdict, and advances the cursor **only**
//! after every listener action of the batch succeeded. Any failure keeps
//! the cursor where it was, so the same batch is replayed on the next run
//! (at-least-once handoff; listeners must tolerate redelivery).
//!
//! Ordering guarantees:
//! - a paused gate returns [`PumpRun::Paused`] before anything is read or
//!   written — the cursor never moves while dispatch is blocked;
//! - an empty log returns [`PumpRun::Idle`] without writing the cursor
//!   (no-op runs leave no trace);
//! - the pump stops at the first event with a failed action: the batch can
//!   no longer advance, so running later events would only add work that
//!   the retry replays anyway. Failed listener ids and the event seq are
//!   reported in [`PumpFailure`].
//!
//! This module owns no scheduling policy and is not wired into any
//! dispatcher; callers decide when to invoke it.

use crate::domain::consumer::{validate_consumer_id, START_CURSOR};
use crate::domain::gate::{allows_dispatch, SchedulerGate};
use crate::domain::listener::ListenerSpec;
use crate::executor::{dispatch_gated, ActionExecutor, DispatchDecision, DispatchOutcome};
use crate::ledger::EventLedger;

/// One pump invocation: what to consume and under which gate.
#[derive(Debug, Clone, Copy)]
pub struct PumpRequest<'a> {
    /// Key-safe consumer id naming the durable cursor (validated).
    pub consumer_id: &'a str,
    /// Gate snapshot consulted before and during dispatch.
    pub gate: &'a SchedulerGate,
    /// Max events handled per run; the caller re-runs for the rest.
    pub batch_limit: u32,
}

/// Verdict of one pump run.
#[derive(Debug)]
pub enum PumpRun {
    /// Gate paused: nothing dispatched, cursor untouched.
    Paused,
    /// No events after the cursor: nothing dispatched, cursor untouched
    /// (no write either).
    Idle,
    /// Every action succeeded and the cursor advanced to `to_seq`;
    /// `events` is the batch size and `actions` the listener actions run.
    /// More events may remain — run the pump again.
    Advanced {
        to_seq: i64,
        events: usize,
        actions: usize,
    },
    /// At least one action failed. The cursor is unchanged (the batch will
    /// be replayed); `failures` lists the failed listener ids per event seq.
    Failed { failures: Vec<PumpFailure> },
}

/// One failed listener action inside a pump batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpFailure {
    /// Seq of the event whose dispatch produced the failure.
    pub event_seq: i64,
    /// Id of the listener whose action failed.
    pub listener_id: String,
    /// Rendered error message of the failed action.
    pub error: String,
}

/// Failures extracted from one event's dispatch outcomes.
fn failures_of(event_seq: i64, outcomes: &[DispatchOutcome]) -> Vec<PumpFailure> {
    outcomes
        .iter()
        .filter_map(|outcome| match &outcome.result {
            Err(err) => Some(PumpFailure {
                event_seq,
                listener_id: outcome.listener_id.clone(),
                error: format!("{err:#}"),
            }),
            Ok(()) => None,
        })
        .collect()
}

/// Runs one durable consume batch. See the module docs for the ordering
/// guarantees; the function itself is side-effect free apart from the
/// ledger reads/writes and listener actions it drives.
pub async fn run_pump(
    ledger: &dyn EventLedger,
    executor: &dyn ActionExecutor,
    registry: &[ListenerSpec],
    req: PumpRequest<'_>,
) -> anyhow::Result<PumpRun> {
    validate_consumer_id(req.consumer_id)
        .map_err(|e| anyhow::anyhow!("consumer id rejected: {e}"))?;
    if !allows_dispatch(req.gate) {
        return Ok(PumpRun::Paused);
    }

    let after = ledger
        .consumer_cursor(req.consumer_id)
        .await?
        .unwrap_or(START_CURSOR);
    let batch = ledger.events_after(after, req.batch_limit).await?;
    if batch.is_empty() {
        return Ok(PumpRun::Idle);
    }

    let mut actions = 0usize;
    for event in &batch {
        match dispatch_gated(executor, registry, event, req.gate).await {
            // The gate value is fixed for this run; matched for totality.
            DispatchDecision::Paused => return Ok(PumpRun::Paused),
            DispatchDecision::Dispatched(outcomes) => {
                let failures = failures_of(event.seq, &outcomes);
                if !failures.is_empty() {
                    return Ok(PumpRun::Failed { failures });
                }
                actions += outcomes.len();
            }
        }
    }

    let to_seq = batch.last().map(|event| event.seq).unwrap_or(after);
    ledger.save_consumer_cursor(req.consumer_id, to_seq).await?;
    Ok(PumpRun::Advanced {
        to_seq,
        events: batch.len(),
        actions,
    })
}
