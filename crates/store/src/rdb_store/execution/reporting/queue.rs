//! Outbox projection + acknowledgement writes (the `execution_report_queue`
//! / `execution_report_acks` / `execution_report_runs` behavior).

use anyhow::{ensure, Result};
use opencoder_core::brain::layered::{
    LayeredEvent, LayeredOperation, LayeredOperationStatus, LayeredPhase, LayeredRun,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::rdb_store::execution::brain_layered;
use crate::rdb_store::execution::reporting::{
    ack_key, queue_index_key, queue_index_prefix, queue_key, queue_row_key, watermark_key,
};
use crate::rdb_store::{kv, tx};
use crate::reporting::{PendingReport, ReportAck};

/// Watermark row encoding (`crate::reporting::ReportRunState` is not
/// serde-derived; the KV row is its JSON form).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct WatermarkRow {
    pub generation: u64,
    pub last_event_seq: u64,
}

impl From<WatermarkRow> for crate::reporting::ReportRunState {
    fn from(row: WatermarkRow) -> Self {
        crate::reporting::ReportRunState {
            generation: row.generation,
            last_event_seq: row.last_event_seq,
        }
    }
}

const GUIDANCE: &str = "layered_guidance";

/// One queued report row (SQL `execution_report_queue` minus the primary
/// key, which lives in the KV key itself; `eligible` is the guidance head).
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct QueueRow {
    pub execution_id: String,
    pub action: String,
    pub item: String,
    pub cursor: i64,
    pub payload: Value,
    #[serde(default = "yes")]
    pub eligible: bool,
}

fn yes() -> bool {
    true
}

impl From<QueueRow> for PendingReport {
    fn from(row: QueueRow) -> Self {
        PendingReport {
            key: queue_key(&row.execution_id, &row.action, &row.item),
            execution_id: row.execution_id,
            action: row.action,
            item: row.item,
            cursor: row.cursor as u64,
            payload: row.payload,
        }
    }
}

/// Every queue row of one execution id, read through the by-execution index.
pub fn queue_rows_of(store: &rdb::store::Store, execution_id: &str) -> Result<Vec<QueueRow>> {
    let prefix = queue_index_prefix(execution_id);
    let mut rows = Vec::new();
    for (index_key, _) in tx::scan_prefix(store, &prefix)? {
        let Some(key) = index_key
            .strip_prefix(prefix.as_slice())
            .and_then(|k| std::str::from_utf8(k).ok())
        else {
            continue;
        };
        if let Some(row) = kv::get_json::<QueueRow>(store, &queue_row_key(key))? {
            rows.push(row);
        }
    }
    Ok(rows)
}

/// Committed + staged guidance cursors per execution, so several guidance
/// events folded into ONE batch still leave exactly one eligible head (the
/// SQL insert sees its own transaction; the KV batch does not).
struct GuidanceHeads<'a> {
    store: &'a rdb::store::Store,
    committed: BTreeMap<String, Vec<i64>>,
    staged: BTreeMap<String, Vec<i64>>,
}

impl<'a> GuidanceHeads<'a> {
    fn new(store: &'a rdb::store::Store) -> Self {
        GuidanceHeads {
            store,
            committed: BTreeMap::new(),
            staged: BTreeMap::new(),
        }
    }

    fn has_smaller(&mut self, execution_id: &str, cursor: i64) -> Result<bool> {
        if !self.committed.contains_key(execution_id) {
            let list = queue_rows_of(self.store, execution_id)?
                .into_iter()
                .filter(|row| row.action == GUIDANCE)
                .map(|row| row.cursor)
                .collect::<Vec<_>>();
            self.committed.insert(execution_id.to_owned(), list);
        }
        let committed = &self.committed[execution_id];
        Ok(committed.iter().any(|c| *c < cursor)
            || self
                .staged
                .get(execution_id)
                .is_some_and(|list| list.iter().any(|c| *c < cursor)))
    }

    fn staged(&mut self, execution_id: &str, cursor: i64) {
        self.staged
            .entry(execution_id.to_owned())
            .or_default()
            .push(cursor);
    }
}

/// One row to fold into the outbox queue.
struct QueueEntry<'a> {
    execution_id: &'a str,
    action: &'a str,
    item: &'a str,
    cursor: u64,
    payload: Value,
}

/// Fold a queue row in: covered-by-ack writes are skipped, guidance heads
/// recompute eligibility against committed and already-staged rows.
fn insert(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    guidance: &mut GuidanceHeads,
    entry: QueueEntry<'_>,
) -> Result<()> {
    ensure!(entry.cursor <= i64::MAX as u64, "report cursor overflow");
    let ack_item = if entry.action == GUIDANCE {
        ""
    } else {
        entry.item
    };
    if let Some(acked) =
        kv::get_json::<i64>(store, &ack_key(entry.execution_id, entry.action, ack_item))?
    {
        if acked >= entry.cursor as i64 {
            return Ok(());
        }
    }
    let eligible = if entry.action == GUIDANCE {
        !guidance.has_smaller(entry.execution_id, entry.cursor as i64)?
    } else {
        true
    };
    let key = queue_key(entry.execution_id, entry.action, entry.item);
    let row = QueueRow {
        execution_id: entry.execution_id.to_owned(),
        action: entry.action.to_owned(),
        item: entry.item.to_owned(),
        cursor: entry.cursor as i64,
        payload: entry.payload,
        eligible,
    };
    tx::put(batch, &queue_row_key(&key), &row)?;
    tx::put_raw(batch, &queue_index_key(entry.execution_id, &key), &[]);
    if entry.action == GUIDANCE {
        guidance.staged(entry.execution_id, entry.cursor as i64);
    }
    Ok(())
}

/// Rebuild the outbox rows of a run inside the caller's (brain-commit)
/// batch; the per-run watermark lands in the same atomic write.
pub fn update(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    run: &LayeredRun,
    operations: &[LayeredOperation],
    events: &[LayeredEvent],
) -> Result<()> {
    let id = run.run_id.as_str();
    for row in queue_rows_of(store, id)? {
        let keep = row.action == GUIDANCE && !run.phase.terminal();
        if !keep {
            let key = queue_key(&row.execution_id, &row.action, &row.item);
            tx::del(batch, &queue_row_key(&key));
            tx::del(batch, &queue_index_key(id, &key));
        }
    }
    let mut guidance = GuidanceHeads::new(store);
    if matches!(run.phase, LayeredPhase::Ready | LayeredPhase::Deciding) {
        insert(
            store,
            batch,
            &mut guidance,
            QueueEntry {
                execution_id: id,
                action: "layered_wake",
                item: "",
                cursor: run.generation,
                payload: json!({"generation": run.generation}),
            },
        )?;
    }
    let dispatch = run.phase == LayeredPhase::Waiting
        || (run.pending_guidance
            && matches!(run.phase, LayeredPhase::Ready | LayeredPhase::Deciding));
    for op in operations {
        if dispatch && op.status == LayeredOperationStatus::Creating && !op.cancel_requested {
            insert(
                store,
                batch,
                &mut guidance,
                QueueEntry {
                    execution_id: id,
                    action: "layered_dispatch",
                    item: &op.operation_id,
                    cursor: 1,
                    payload: json!(op),
                },
            )?;
        }
        if !run.phase.terminal() && op.cancel_requested && !op.status.terminal() {
            insert(
                store,
                batch,
                &mut guidance,
                QueueEntry {
                    execution_id: id,
                    action: "layered_cancel",
                    item: &op.operation_id,
                    cursor: 1,
                    payload: json!(op),
                },
            )?;
        }
    }
    if run.phase.terminal() {
        insert(
            store,
            batch,
            &mut guidance,
            QueueEntry {
                execution_id: id,
                action: "settle",
                item: "",
                cursor: run.generation,
                payload: Value::Null,
            },
        )?;
    } else {
        for event in events
            .iter()
            .filter(|e| e.event_type == "guidance_processed" && !e.guidance.is_empty())
        {
            insert(
                store,
                batch,
                &mut guidance,
                QueueEntry {
                    execution_id: id,
                    action: GUIDANCE,
                    item: &format!("{:020}", event.seq),
                    cursor: event.seq,
                    payload: json!({"seq": event.seq, "guidance": event.guidance}),
                },
            )?;
        }
    }
    tx::put(
        batch,
        &watermark_key(id),
        &WatermarkRow {
            generation: run.generation,
            last_event_seq: run.last_event_seq,
        },
    )
}

/// Idempotent acknowledgement: a replayed or stale ACK writes nothing; a
/// live one drops the covered queue rows and promotes the next guidance
/// head — atomically, or not at all.
pub fn acknowledge(store: &rdb::store::Store, ack: &ReportAck) -> Result<()> {
    ensure!(
        ack.cursor <= i64::MAX as u64,
        "report acknowledgement overflow"
    );
    let key = ack_key(&ack.execution_id, &ack.action, &ack.item);
    if let Some(acked) = kv::get_json::<i64>(store, &key)? {
        if acked >= ack.cursor as i64 {
            return Ok(());
        }
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &(ack.cursor as i64))?;
    let rows = queue_rows_of(store, &ack.execution_id)?;
    for row in &rows {
        let item_matches = ack.action == GUIDANCE || row.item == ack.item;
        if row.action == ack.action && item_matches && row.cursor <= ack.cursor as i64 {
            let key = queue_key(&row.execution_id, &row.action, &row.item);
            tx::del(&mut batch, &queue_row_key(&key));
            tx::del(&mut batch, &queue_index_key(&row.execution_id, &key));
        }
    }
    if ack.action == GUIDANCE {
        let mut head = rows
            .into_iter()
            .filter(|row| row.action == GUIDANCE && row.cursor > ack.cursor as i64)
            .min_by_key(|row| row.cursor);
        if let Some(row) = head.as_mut() {
            row.eligible = true;
            let key = queue_key(&row.execution_id, &row.action, &row.item);
            tx::put(&mut batch, &queue_row_key(&key), row)?;
        }
    }
    tx::commit(store, batch)
}

/// One startup reconciliation pass: reproject every run whose outbox
/// watermark is missing or behind its committed generation.
pub fn recover(store: &rdb::store::Store) -> Result<()> {
    loop {
        let mut ids = Vec::new();
        for (_, value) in tx::scan_prefix(store, b"blr/")? {
            let run: LayeredRun = kv::decode(&value)?;
            let watermark = kv::get_json::<WatermarkRow>(store, &watermark_key(&run.run_id))?;
            if watermark.map(|w| w.generation) != Some(run.generation) {
                ids.push(run.run_id.clone());
            }
            if ids.len() >= 128 {
                break;
            }
        }
        if ids.is_empty() {
            return Ok(());
        }
        let mut batch = rocksdb::WriteBatch::default();
        for id in &ids {
            let Some(snapshot) = brain_layered::load(store, id)? else {
                continue;
            };
            // The whole guidance history re-projects; bounded paging would
            // silently drop receipts on long-running runs.
            let mut events = Vec::new();
            for (_, value) in tx::scan_prefix(store, &brain_layered::event_prefix(id))? {
                let event: LayeredEvent = kv::decode(&value)?;
                if event.event_type == "guidance_processed" {
                    events.push(event);
                }
            }
            update(
                store,
                &mut batch,
                &snapshot.run,
                &snapshot.operations,
                &events,
            )?;
        }
        tx::commit(store, batch)?;
    }
}
