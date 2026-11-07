//! Fleet schema bootstrap.
//!
//! The KV engine needs no DDL; the marker row stamps the logical layout
//! version so a future layout change can detect and migrate old databases
//! instead of silently mixing row shapes.

use super::keys;
use crate::rdb_store::kv;
use anyhow::Result;

const LAYOUT_VERSION: i64 = 1;

pub(super) fn initialize(store: &rdb::store::Store) -> Result<()> {
    if kv::get_json::<i64>(store, &keys::schema_key())? != Some(LAYOUT_VERSION) {
        let mut batch = rocksdb::WriteBatch::default();
        batch.put(keys::schema_key(), kv::encode(&LAYOUT_VERSION)?);
        crate::rdb_store::tx::commit(store, batch)?;
    }
    Ok(())
}
