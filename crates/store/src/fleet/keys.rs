//! Fleet control database KV layout and shared row codecs.
//!
//! Physical key families (all UTF-8 except the fixed-width order bytes;
//! identifiers never contain `/`):
//! - `node/<id>`                    node registration row
//! - `def/<kind>/<id>`              definitions KV (raw inventory-encoded body)
//! - `bpv/<plan>/<version:020>`     immutable brain plan version bodies
//! - `xrow/<id>`                    five-column execution index row
//! - `xall/<ord8>/<id>`             global order: created_at DESC, id ASC
//! - `xnk/<node>/<kind>/<ord8>/<id>` composite order for filtered pages
//! - `rcp/<scope>/<id>`             dispatch receipts
//! - `asg/<id>`                     frozen execution assignments
//! - `prk/<dispatch-key>`           project run-key → assignment id
//! - `wm/<node>`                    node report watermark
//! - `rt/<id>` / `rta/`             host runtimes + active marker (UNIQUE)
//! - `own/<execution>`              runtime owner (UNIQUE per execution)
//! - `cap/`                         host capacity singleton
//! - `cq/<seq:020>`                 capacity queue rows (FIFO by seq)
//! - `cqt/<ticket>` / `cql/<exec>`  capacity ticket / live-slot unique indexes
//! - `seq/<family>`                 persisted counters (shared builder)
//! - `sch/fleet`                    schema bootstrap marker

use crate::rdb_store::kv::{self, pad_ord};
use anyhow::Result;
use opencoder_core::fleet::ExecutionIndex;

pub(super) const XALL_PREFIX: &[u8] = b"xall/";
const ORD_BYTES: usize = 8;

pub(super) fn schema_key() -> Vec<u8> {
    b"sch/fleet".to_vec()
}

pub(super) fn node_key(id: &str) -> Vec<u8> {
    format!("node/{id}").into_bytes()
}

pub(super) fn node_prefix() -> Vec<u8> {
    b"node/".to_vec()
}

pub(super) fn def_prefix(kind: &str) -> Vec<u8> {
    format!("def/{kind}/").into_bytes()
}

pub(super) fn def_key(kind: &str, id: &str) -> Vec<u8> {
    let mut key = def_prefix(kind);
    key.extend_from_slice(id.as_bytes());
    key
}

pub(super) fn plan_prefix(id: &str) -> Vec<u8> {
    format!("bpv/{id}/").into_bytes()
}

pub(super) fn plan_key(id: &str, version: u64) -> Vec<u8> {
    let mut key = plan_prefix(id);
    key.extend_from_slice(pad_ord(version as i64).as_bytes());
    key
}

pub(super) fn xrow_key(id: &str) -> Vec<u8> {
    format!("xrow/{id}").into_bytes()
}

/// Order-inverting encoding: ascending `i64` maps to descending big-endian
/// bytes, so a forward scan of `xall/` yields `created_at DESC`.
pub(super) fn desc_ord(created_at: i64) -> [u8; ORD_BYTES] {
    (created_at ^ i64::MAX).to_be_bytes()
}

pub(super) fn xall_key(created_at: i64, id: &str) -> Vec<u8> {
    let mut key = XALL_PREFIX.to_vec();
    key.extend_from_slice(&desc_ord(created_at));
    key.extend_from_slice(id.as_bytes());
    key
}

/// Scan start strictly after `(created_at, id)` in the global order.
pub(super) fn after_xall(created_at: i64, id: &str) -> Vec<u8> {
    let mut key = xall_key(created_at, id);
    key.push(0);
    key
}

/// Parse the id suffix of an `xall`/`xnk` order key.
pub(super) fn ord_key_id(prefix: &[u8], key: &[u8]) -> Option<String> {
    let suffix = key.strip_prefix(prefix)?;
    if suffix.len() <= ORD_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&suffix[ORD_BYTES..]).into_owned())
}

pub(super) fn xnk_prefix(node_id: &str, kind: &str) -> Vec<u8> {
    format!("xnk/{node_id}/{kind}/").into_bytes()
}

pub(super) fn xnk_key(node_id: &str, kind: &str, created_at: i64, id: &str) -> Vec<u8> {
    let mut key = xnk_prefix(node_id, kind);
    key.extend_from_slice(&desc_ord(created_at));
    key.extend_from_slice(id.as_bytes());
    key
}

pub(super) fn rcp_prefix(scope: &str) -> Vec<u8> {
    format!("rcp/{scope}/").into_bytes()
}

pub(super) fn rcp_key(scope: &str, id: &str) -> Vec<u8> {
    let mut key = rcp_prefix(scope);
    key.extend_from_slice(id.as_bytes());
    key
}

/// Receipt id from a full `rcp/<scope>/<id>` key.
pub(super) fn rcp_id(key: &[u8]) -> String {
    std::str::from_utf8(key)
        .unwrap_or_default()
        .splitn(3, '/')
        .nth(2)
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn asg_key(id: &str) -> Vec<u8> {
    format!("asg/{id}").into_bytes()
}

pub(super) fn prk_key(dispatch_key: &str) -> Vec<u8> {
    format!("prk/{dispatch_key}").into_bytes()
}

pub(super) fn wm_key(node_id: &str) -> Vec<u8> {
    format!("wm/{node_id}").into_bytes()
}

pub(super) fn rt_key(id: &str) -> Vec<u8> {
    format!("rt/{id}").into_bytes()
}

pub(super) fn rt_active_key() -> Vec<u8> {
    b"rta/".to_vec()
}

pub(super) fn own_key(execution_id: &str) -> Vec<u8> {
    format!("own/{execution_id}").into_bytes()
}

pub(super) fn cap_key() -> Vec<u8> {
    b"cap/".to_vec()
}

pub(super) fn cq_prefix() -> Vec<u8> {
    b"cq/".to_vec()
}

pub(super) fn cq_key(sequence: i64) -> Vec<u8> {
    let mut key = cq_prefix();
    key.extend_from_slice(pad_ord(sequence).as_bytes());
    key
}

pub(super) fn cq_ticket_index(ticket: &str) -> Vec<u8> {
    format!("cqt/{ticket}").into_bytes()
}

pub(super) fn cq_live_index(execution_id: &str) -> Vec<u8> {
    format!("cql/{execution_id}").into_bytes()
}

pub(super) fn read_index(store: &rdb::store::Store, id: &str) -> Result<Option<ExecutionIndex>> {
    kv::get_json(store, &xrow_key(id))
}

/// Stage the row plus both order-index entries for a NEW execution index row.
/// Index keys are derived from immutable fields, so status-only updates skip
/// this and rewrite just the row.
pub(super) fn put_index_row(
    batch: &mut rocksdb::WriteBatch,
    record: &ExecutionIndex,
) -> Result<()> {
    batch.put(xrow_key(&record.id), kv::encode(record)?);
    batch.put(xall_key(record.created_at, &record.id), b"");
    batch.put(
        xnk_key(
            &record.node_id,
            record.kind.prefix(),
            record.created_at,
            &record.id,
        ),
        b"",
    );
    Ok(())
}

/// Stage an atomic prefix-range tombstone inside `batch` (`ON DELETE
/// CASCADE` for a whole child family, committed with the batch).
pub(super) fn stage_delete_prefix(batch: &mut rocksdb::WriteBatch, prefix: &[u8]) {
    let upper = kv::prefix_upper(prefix);
    batch.delete_range(prefix, &upper);
}
