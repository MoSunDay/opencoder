//! A bounded prefix on the stored definition row avoids shared-schema
//! migrations. Old writers discard the optional field; the next read safely
//! stamps it again.
use super::keys;
use super::FleetStore;
use crate::rdb_store::tx;
use anyhow::{ensure, Result};
use serde_json::Value;

const FIELD: &str = "_opencoder_inventory_revision";
const PREFIX: &str = "{\"_opencoder_inventory_revision\":\"";
const PREFIX_BYTES: usize = 96;
const KIND: &str = "runtime_sleep";

pub(super) fn encode(kind: &str, body: &Value) -> Result<Vec<u8>> {
    if kind == KIND && body.is_object() {
        let encoded = if body.get(FIELD).is_some() {
            let mut body = body.clone();
            body.as_object_mut().unwrap().remove(FIELD);
            serde_json::to_vec(&body)?
        } else {
            serde_json::to_vec(body)?
        };
        // The stamped prefix is `{"<FIELD>":"<ulid>"` glued onto the body
        // object's tail (its opening brace is replaced by the prefix).
        let mut stamped = format!("{PREFIX}{}", ulid::Ulid::new());
        stamped.push('"');
        stamped.push_str(std::str::from_utf8(&encoded[1..])?);
        return Ok(stamped.into_bytes());
    }
    Ok(serde_json::to_vec(body)?)
}

pub(super) fn decode(kind: &str, encoded: &str) -> Result<Value> {
    let mut body: Value = serde_json::from_str(encoded)?;
    if kind == KIND {
        if let Some(object) = body.as_object_mut() {
            object.remove(FIELD);
        }
    }
    Ok(body)
}

fn stamp(prefix: Option<&str>) -> Option<(String, bool)> {
    match prefix {
        None | Some("null") => Some((String::new(), false)),
        Some(value) => {
            let revision = value.strip_prefix(PREFIX)?.split_once("\",")?.0;
            ulid::Ulid::from_string(revision).ok()?;
            Some((revision.to_owned(), true))
        }
    }
}

fn raw_row(store: &rdb::store::Store, id: &str) -> Result<Option<String>> {
    match rdb::store::get(store, b"", &keys::def_key(KIND, id))
        .map_err(crate::rdb_store::kv::engine_err)?
    {
        Some(bytes) => Ok(Some(String::from_utf8(bytes)?)),
        None => Ok(None),
    }
}

/// Lease-free fast path: `None` when the row needs re-stamping.
pub(super) fn stamped(store: &rdb::store::Store, id: &str) -> Result<Option<(String, bool)>> {
    Ok(stamp(raw_row(store, id)?.as_deref().map(prefix_window)))
}

/// The durable revision marker for one frozen runtime inventory. Rows written
/// by an older writer are re-stamped; the caller owns the write lease.
pub(super) fn runtime_sleep_stamp(store: &rdb::store::Store, id: &str) -> Result<(String, bool)> {
    if let Some(stamp) = stamped(store, id)? {
        return Ok(stamp);
    }
    let row = raw_row(store, id)?;
    let body: Value = serde_json::from_str(row.as_deref().unwrap_or("null"))?;
    ensure!(body.is_object(), "invalid frozen runtime inventory");
    let encoded = encode(KIND, &body)?;
    let result = stamp(Some(prefix_window(std::str::from_utf8(&encoded)?)))
        .expect("newly stamped inventory");
    let mut batch = rocksdb::WriteBatch::default();
    batch.put(keys::def_key(KIND, id), encoded);
    tx::commit(store, batch)?;
    Ok(result)
}

fn prefix_window(row: &str) -> &str {
    row.get(..PREFIX_BYTES).unwrap_or(row)
}

#[path = "access/inventory_meta.rs"]
mod database_access;
