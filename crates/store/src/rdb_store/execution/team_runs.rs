//! Team topic-run persistence (`team_topic_runs`) over the rdb KV engine —
//! the (topic, node) fan-out ledger of the opencoder-team runtime. Ported
//! from the SQL backend's `execution/team_runs`.
//!
//! Key family: `trun/<topic_id>/<node_id>` — the key IS the SQL primary
//! key, so upserts target one row. The value wraps the record with a
//! `row_ord` (global counter, the `rowid` stand-in) that breaks same-ms
//! `created_at` ties in listings. ULIDs never order anything.
//!
//! SQL parity notes:
//! - FK `nodes(id) ON DELETE CASCADE`: `nodes::delete_node` owns its own
//!   cascade and cannot know this table, so the delete side is realized
//!   here as GC-on-list (rows whose node is gone are purged and hidden);
//!   the insert side is enforced eagerly (upsert refuses unknown nodes,
//!   matching `FOREIGN KEY constraint failed`).

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::rdb_store::execution::nodes;
use crate::rdb_store::kv::{self};
use crate::rdb_store::{seq, tx};
use crate::team_types::TeamTopicRunRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrunRow {
    rec: TeamTopicRunRecord,
    row_ord: i64,
}

fn topic_prefix(topic_id: &str) -> Vec<u8> {
    format!("trun/{topic_id}/").into_bytes()
}

fn run_key(topic_id: &str, node_id: &str) -> Vec<u8> {
    format!("trun/{topic_id}/{node_id}").into_bytes()
}

/// Insert or refresh one `(topic_id, node_id)` run row. A NEW row takes the
/// record verbatim; an EXISTING row keeps its original `created_at` (and
/// ordering) and updates ONLY `status` — a refresh never restarts the run's
/// clock. Unknown nodes are rejected (FK parity).
pub fn upsert(store: &rdb::store::Store, rec: &TeamTopicRunRecord) -> Result<()> {
    if kv::get_json::<crate::types::NodeRecord>(store, &nodes::node_key(&rec.node_id))?.is_none() {
        bail!(
            "upsert team topic run: node {:?} does not exist (fk)",
            rec.node_id
        );
    }
    let key = run_key(&rec.topic_id, &rec.node_id);
    let mut batch = rocksdb::WriteBatch::default();
    let row = match kv::get_json::<TrunRow>(store, &key)? {
        Some(existing) => TrunRow {
            rec: TeamTopicRunRecord {
                status: rec.status.clone(),
                ..existing.rec
            },
            row_ord: existing.row_ord,
        },
        None => {
            let row_ord = seq::next_global(store, &mut batch, "trun-row")?;
            TrunRow {
                rec: rec.clone(),
                row_ord,
            }
        }
    };
    tx::put(&mut batch, &key, &row)?;
    tx::commit(store, batch)
}

/// Flip EVERY row of `topic_id` to `finished` (one batch). No-op for
/// unknown or already-finished topics — "nothing to finish" is success.
pub fn finish(store: &rdb::store::Store, topic_id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    for (key, value) in tx::scan_prefix(store, &topic_prefix(topic_id))? {
        let mut row = kv::decode::<TrunRow>(&value)?;
        row.rec.status = crate::team_types::TEAM_RUN_FINISHED.to_string();
        tx::put(&mut batch, &key, &row)?;
    }
    tx::commit(store, batch)
}

/// All run rows of `topic_id`, oldest `(created_at, row_ord)` first. Dead
/// rows (their node was deleted) are purged first — the observable half of
/// the SQL cascade — and never returned.
pub fn list(store: &rdb::store::Store, topic_id: &str) -> Result<Vec<TeamTopicRunRecord>> {
    let mut rows: Vec<TrunRow> = Vec::new();
    let mut dead: Vec<Vec<u8>> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &topic_prefix(topic_id))? {
        let row = kv::decode::<TrunRow>(&value)?;
        if kv::get_json::<crate::types::NodeRecord>(store, &nodes::node_key(&row.rec.node_id))?
            .is_none()
        {
            dead.push(key);
        } else {
            rows.push(row);
        }
    }
    if !dead.is_empty() {
        let mut batch = rocksdb::WriteBatch::default();
        for key in &dead {
            tx::del(&mut batch, key);
        }
        tx::commit(store, batch)?;
    }
    rows.sort_by(|a, b| {
        a.rec
            .created_at
            .cmp(&b.rec.created_at)
            .then(a.row_ord.cmp(&b.row_ord))
    });
    Ok(rows.into_iter().map(|r| r.rec).collect())
}
