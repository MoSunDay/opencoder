//! Session inputs (`inp/<session>/<seq:020>`): admission with idempotency,
//! FIFO promotion/claim, the recorded lifecycle and queue reordering.
//!
//! `seq` is a global counter (`seq/inp`), matching the SQL rowid contract
//! that `delete_input(seq)` addresses a row without a session. `admitted_seq`
//! is per-session dense (MAX+1 under the lease). A unique dedup index
//! (`idx/inp-dedup/<session>/<input id>`) backs `admit_input_once`: a retry
//! with the same semantic payload returns the original seq; a different
//! payload rejects with the same typed `InputConflict`.

use anyhow::{bail, Context, Result};

use crate::rdb_store::kv::{
    self, decode, input_dedup_index, input_key, input_prefix, input_seq_index,
};
use crate::rdb_store::{index, seq, tx};
use crate::types::{Delivery, InputAdmission, InputConflict, SessionInput};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct InputRow {
    pub seq: i64,
    pub id: String,
    pub session_id: String,
    pub delivery: String,
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_text: Option<String>,
    pub admitted_seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_seq: Option<i64>,
    #[serde(default)]
    pub recorded: bool,
}

impl InputRow {
    pub fn to_input(&self) -> SessionInput {
        SessionInput {
            seq: Some(self.seq),
            id: self.id.clone(),
            session_id: self.session_id.clone(),
            delivery: Delivery::parse(&self.delivery).unwrap_or_default(),
            prompt: self.prompt.clone(),
            images: self.images.clone(),
            display_text: self.display_text.clone(),
            admitted_seq: self.admitted_seq,
            promoted_seq: self.promoted_seq,
        }
    }
}

/// All rows of one session, ordered by `admitted_seq` (the FIFO order).
pub fn rows_of(store: &rdb::store::Store, session_id: &str) -> Result<Vec<InputRow>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, &input_prefix(session_id))? {
        rows.push(decode::<InputRow>(&value)?);
    }
    rows.sort_by_key(|r| r.admitted_seq);
    Ok(rows)
}

fn max_session_seq(rows: &[InputRow]) -> i64 {
    rows.iter().map(|r| r.seq).max().unwrap_or(0)
}

fn next_admitted_seq(rows: &[InputRow]) -> i64 {
    rows.iter().map(|r| r.admitted_seq).max().unwrap_or(0) + 1
}

fn stage_row(batch: &mut rocksdb::WriteBatch, row: &InputRow) -> Result<()> {
    tx::put(batch, &input_key(&row.session_id, row.seq), row)?;
    tx::put_raw(batch, &input_seq_index(row.seq), row.session_id.as_bytes());
    index::put_i64(batch, &input_dedup_index(&row.session_id, &row.id), row.seq)?;
    Ok(())
}

/// Plain admission. Duplicate `(session_id, id)` rejects with the SQL
/// backend's UNIQUE-constraint failure text; a missing session rejects
/// with the SQL FOREIGN KEY constraint failure (replicated contract).
pub fn admit(store: &rdb::store::Store, input: &SessionInput) -> Result<i64> {
    let rows = rows_of(store, &input.session_id)?;
    if rows.iter().any(|r| r.id == input.id) {
        bail!(
            "insert input: UNIQUE constraint failed: session_inputs.session_id, session_inputs.id"
        );
    }
    if super::sessions::get(store, &input.session_id)?.is_none() {
        bail!("insert input: FOREIGN KEY constraint failed");
    }
    let mut batch = rocksdb::WriteBatch::default();
    let row = new_row(store, &mut batch, input, next_admitted_seq(&rows))?;
    stage_row(&mut batch, &row)?;
    tx::commit(store, batch)?;
    Ok(row.seq)
}

fn new_row(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    input: &SessionInput,
    admitted_seq: i64,
) -> Result<InputRow> {
    Ok(InputRow {
        seq: seq::next_global(store, batch, "inp")?,
        id: input.id.clone(),
        session_id: input.session_id.clone(),
        delivery: input.delivery.as_str().to_string(),
        prompt: input.prompt.clone(),
        images: input.images.clone(),
        display_text: input.display_text.clone(),
        admitted_seq,
        promoted_seq: None,
        recorded: false,
    })
}

/// Atomic idempotent admission keyed by `(session_id, id)`. An insert
/// path (dedup miss) into a missing session rejects with the SQL
/// FOREIGN KEY constraint failure (replicated contract).
pub fn admit_once(store: &rdb::store::Store, input: &SessionInput) -> Result<InputAdmission> {
    let dedup = input_dedup_index(&input.session_id, &input.id);
    let mut batch = rocksdb::WriteBatch::default();
    if let Some(existing_seq) = index::owner_i64(store, &dedup)? {
        let row = row_by_seq(store, existing_seq)?.context("idempotent input row missing")?;
        if !same_semantic_payload(&row, input) {
            bail!(InputConflict {
                session_id: input.session_id.clone(),
                input_id: input.id.clone(),
            });
        }
        return Ok(InputAdmission {
            seq: row.seq,
            inserted: false,
        });
    }
    if super::sessions::get(store, &input.session_id)?.is_none() {
        bail!("insert input: FOREIGN KEY constraint failed");
    }
    let rows = rows_of(store, &input.session_id)?;
    let row = new_row(store, &mut batch, input, next_admitted_seq(&rows))?;
    stage_row(&mut batch, &row)?;
    tx::commit(store, batch)?;
    Ok(InputAdmission {
        seq: row.seq,
        inserted: true,
    })
}

fn same_semantic_payload(row: &InputRow, input: &SessionInput) -> bool {
    row.delivery == input.delivery.as_str()
        && row.prompt == input.prompt
        && row.images == input.images
        && row.display_text == input.display_text
}

fn row_by_seq(store: &rdb::store::Store, s: i64) -> Result<Option<InputRow>> {
    // The seq index maps global seq -> session id; fall back to the row key.
    let session = index::owner_str(store, &input_seq_index(s))?;
    let Some(session) = session else {
        return Ok(None);
    };
    get_row(store, &session, s)
}

fn get_row(store: &rdb::store::Store, session_id: &str, s: i64) -> Result<Option<InputRow>> {
    kv::get_json(store, &input_key(session_id, s))
}

/// Pending (unpromoted) rows of one delivery, FIFO order.
pub fn pending(
    store: &rdb::store::Store,
    session_id: &str,
    delivery: Delivery,
) -> Result<Vec<SessionInput>> {
    Ok(rows_of(store, session_id)?
        .into_iter()
        .filter(|r| r.promoted_seq.is_none() && r.delivery == delivery.as_str())
        .map(|r| r.to_input())
        .collect())
}

/// Rewrite one row in place (same key), staging into the caller's batch.
fn stage_update(batch: &mut rocksdb::WriteBatch, row: &InputRow) -> Result<()> {
    tx::put(batch, &input_key(&row.session_id, row.seq), row)
}

/// Promote all pending rows of `delivery` up to `up_to_admitted_seq`.
pub fn promote(
    store: &rdb::store::Store,
    session_id: &str,
    up_to_admitted_seq: i64,
    delivery: Delivery,
) -> Result<Vec<i64>> {
    let rows = rows_of(store, session_id)?;
    let targets: Vec<InputRow> = rows
        .iter()
        .filter(|r| {
            r.promoted_seq.is_none()
                && r.delivery == delivery.as_str()
                && r.admitted_seq <= up_to_admitted_seq
        })
        .cloned()
        .collect();
    let mut batch = rocksdb::WriteBatch::default();
    let promoted_seq = max_session_seq(&rows) + 1;
    let mut seqs = Vec::with_capacity(targets.len());
    for mut row in targets {
        row.promoted_seq = Some(promoted_seq);
        // Repromotion resets the consume marker (recovery semantics).
        row.recorded = false;
        seqs.push(row.seq);
        stage_update(&mut batch, &row)?;
    }
    tx::commit(store, batch)?;
    Ok(seqs)
}

/// Oldest pending queued row not yet consumed into the transcript.
fn oldest_claimable(rows: &[InputRow]) -> Option<InputRow> {
    rows.iter()
        .filter(|r| r.delivery == "queue" && r.promoted_seq.is_none() && !r.recorded)
        .min_by_key(|r| r.admitted_seq)
        .cloned()
}

pub fn promote_next_queued(store: &rdb::store::Store, session_id: &str) -> Result<Option<i64>> {
    let rows = rows_of(store, session_id)?;
    let Some(mut target) = oldest_claimable(&rows) else {
        return Ok(None);
    };
    let mut batch = rocksdb::WriteBatch::default();
    target.promoted_seq = Some(max_session_seq(&rows) + 1);
    stage_update(&mut batch, &target)?;
    tx::commit(store, batch)?;
    Ok(Some(target.seq))
}

/// Claim the oldest queued row and return it with its promoted state.
pub fn claim_next_queue(
    store: &rdb::store::Store,
    session_id: &str,
) -> Result<Option<(i64, SessionInput)>> {
    let rows = rows_of(store, session_id)?;
    let Some(mut target) = oldest_claimable(&rows) else {
        return Ok(None);
    };
    let mut batch = rocksdb::WriteBatch::default();
    target.promoted_seq = Some(max_session_seq(&rows) + 1);
    stage_update(&mut batch, &target)?;
    tx::commit(store, batch)?;
    let seq = target.seq;
    Ok(Some((seq, target.to_input())))
}

/// Reset promoted rows back to pending; only touches promoted rows.
pub fn unpromote(store: &rdb::store::Store, session_id: &str, seqs: &[i64]) -> Result<()> {
    if seqs.is_empty() {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    for s in seqs {
        if let Some(mut row) = get_row(store, session_id, *s)? {
            if row.promoted_seq.is_some() {
                row.promoted_seq = None;
                stage_update(&mut batch, &row)?;
            }
        }
    }
    tx::commit(store, batch)
}

/// Mark rows as durably consumed. Idempotent.
pub fn mark_recorded(store: &rdb::store::Store, session_id: &str, seqs: &[i64]) -> Result<()> {
    if seqs.is_empty() {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    for s in seqs {
        if let Some(mut row) = get_row(store, session_id, *s)? {
            row.recorded = true;
            stage_update(&mut batch, &row)?;
        }
    }
    tx::commit(store, batch)
}

/// Flip promoted-but-unrecorded rows back to pending; returns the count.
pub fn recover_orphans(store: &rdb::store::Store, session_id: &str) -> Result<u64> {
    let mut batch = rocksdb::WriteBatch::default();
    let mut recovered = 0u64;
    for mut row in rows_of(store, session_id)? {
        if row.promoted_seq.is_some() && !row.recorded {
            row.promoted_seq = None;
            row.recorded = false;
            stage_update(&mut batch, &row)?;
            recovered += 1;
        }
    }
    tx::commit(store, batch)?;
    Ok(recovered)
}

/// Delete a pending input by global seq. Idempotent; promoted rows stay.
pub fn delete_input(store: &rdb::store::Store, s: i64) -> Result<()> {
    let Some(row) = row_by_seq(store, s)? else {
        return Ok(());
    };
    if row.promoted_seq.is_some() {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &input_key(&row.session_id, row.seq));
    tx::del(&mut batch, &input_seq_index(row.seq));
    tx::del(&mut batch, &input_dedup_index(&row.session_id, &row.id));
    tx::commit(store, batch)
}

/// Swap the FIFO order of two pending rows by exchanging `admitted_seq`.
pub fn swap_input_order(
    store: &rdb::store::Store,
    session_id: &str,
    seq_a: i64,
    seq_b: i64,
) -> Result<()> {
    if seq_a == seq_b {
        return Ok(());
    }
    let mut a = take_pending(store, session_id, seq_a)?;
    let mut b = take_pending(store, session_id, seq_b)?;
    std::mem::swap(&mut a.admitted_seq, &mut b.admitted_seq);
    let mut batch = rocksdb::WriteBatch::default();
    stage_update(&mut batch, &a)?;
    stage_update(&mut batch, &b)?;
    tx::commit(store, batch)
}

fn take_pending(store: &rdb::store::Store, session_id: &str, s: i64) -> Result<InputRow> {
    get_row(store, session_id, s)?
        .filter(|r| r.promoted_seq.is_none())
        .ok_or_else(|| {
            anyhow::anyhow!("input seq {s} not found, not in session, or already promoted")
        })
}
