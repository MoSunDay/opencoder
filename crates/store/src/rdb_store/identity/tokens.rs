//! Access tokens (`platform_tokens`) over the rdb KV engine.
//!
//! Key families:
//! - `tok/<id>`  token row (metadata + the digest itself)
//! - `idx/tok-hash/<hash>` → token row key  (UNIQUE token_hash)
//! - `idx/tok-user/<user>/<id>` → token row key  (per-user membership,
//!   backs the user-delete cascade)
//!
//! Hash-index maintenance lives in the same batch as the row write, so a
//! conflict aborts before anything commits — the SQL UNIQUE behavior.

use anyhow::{bail, Result};
use opencoder_core::identity::Role;
use opencoder_core::message::now_ms;
use serde::{Deserialize, Serialize};

use crate::rdb_store::index;
use crate::rdb_store::kv;
use crate::rdb_store::tx;
use crate::users::PlatformUser;

/// Label of the token row `create_user` seeds for a new user.
pub(super) const INITIAL_TOKEN_LABEL: &str = "初始 Token";

pub fn token_key(id: &str) -> Vec<u8> {
    format!("tok/{id}").into_bytes()
}

fn hash_index(token_hash: &str) -> Vec<u8> {
    format!("idx/tok-hash/{token_hash}").into_bytes()
}

fn user_index(user_name: &str, id: &str) -> Vec<u8> {
    format!("idx/tok-user/{user_name}/{id}").into_bytes()
}

fn user_index_prefix(user_name: &str) -> Vec<u8> {
    format!("idx/tok-user/{user_name}/").into_bytes()
}

/// One `platform_tokens` row.
#[derive(Clone, Serialize, Deserialize)]
struct TokenRow {
    #[serde(flatten)]
    token: crate::AccessToken,
    token_hash: String,
}

/// Stage a token insert (row + both indexes) into `batch`. The UNIQUE
/// constraints are checked under the lease; the owner-user foreign key is
/// the caller's duty (the user row may be staged in the same batch, as
/// `create_user` does).
pub(super) fn stage_create(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    token: &crate::AccessToken,
    token_hash: &str,
) -> Result<()> {
    let key = token_key(&token.id);
    if kv::get_json::<TokenRow>(store, &key)?.is_some() {
        bail!("insert platform token: UNIQUE constraint failed: platform_tokens.id");
    }
    index::claim_unique(
        store,
        batch,
        &hash_index(token_hash),
        &key,
        "platform_tokens.token_hash",
    )?;
    tx::put(
        batch,
        &key,
        &TokenRow {
            token: token.clone(),
            token_hash: token_hash.to_string(),
        },
    )?;
    index::put(batch, &user_index(&token.user_name, &token.id), &key);
    Ok(())
}

/// Insert a token on its own (single-row transaction in the SQL backend).
/// The owner-user foreign key is checked here; callers staging the token
/// into a batch that already carries the user row use [`stage_create`].
pub fn create(
    store: &rdb::store::Store,
    token: &crate::AccessToken,
    token_hash: &str,
) -> Result<()> {
    if super::find_by_name(store, &token.user_name)?.is_none() {
        bail!("insert platform token: FOREIGN KEY constraint failed");
    }
    let mut batch = rocksdb::WriteBatch::default();
    stage_create(store, &mut batch, token, token_hash)?;
    tx::commit(store, batch)
}

/// Every token, newest first (`created_at DESC, id DESC`).
pub fn list(store: &rdb::store::Store) -> Result<Vec<crate::AccessToken>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"tok/")? {
        rows.push(kv::decode::<TokenRow>(&value)?.token);
    }
    rows.sort_by(|a, b| {
        let x = (b.created_at, &b.id);
        let y = (a.created_at, &a.id);
        x.cmp(&y)
    });
    Ok(rows)
}

/// Digest → live user. Revoked tokens never match; expiry is checked
/// against the server clock like the SQL `(expires_at IS NULL OR ...)`.
pub(super) fn find_user_by_hash(
    store: &rdb::store::Store,
    token_hash: &str,
) -> Result<Option<PlatformUser>> {
    let Some(owner) = index::owner_str(store, &hash_index(token_hash))? else {
        return Ok(None);
    };
    let Some(row) = kv::get_json::<TokenRow>(store, owner.as_bytes())? else {
        return Ok(None);
    };
    if row.token.revoked_at.is_some() {
        return Ok(None);
    }
    if let Some(expires_at) = row.token.expires_at {
        if expires_at <= now_ms() {
            return Ok(None);
        }
    }
    super::find_by_name(store, &row.token.user_name)
}

/// Re-point a user's initial token (`user:<name>` row) at a new digest;
/// `Ok(false)` when that row does not exist. A hash owned by a different
/// token row rejects with the UNIQUE-constraint failure text.
pub(super) fn update_hash(store: &rdb::store::Store, name: &str, token_hash: &str) -> Result<bool> {
    let key = token_key(&format!("user:{name}"));
    let Some(mut row) = kv::get_json::<TokenRow>(store, &key)? else {
        return Ok(false);
    };
    let mut batch = rocksdb::WriteBatch::default();
    index::claim_unique(
        store,
        &mut batch,
        &hash_index(token_hash),
        &key,
        "platform_tokens.token_hash",
    )?;
    if row.token_hash != token_hash {
        index::remove(&mut batch, &hash_index(&row.token_hash));
    }
    row.token_hash = token_hash.to_string();
    tx::put(&mut batch, &key, &row)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Revoke by id (`revoked_at = COALESCE(revoked_at, now)`): an existing
/// stamp is kept, and admin-credential tokens are not revocable. Returns
/// whether the row matched and was revocable at all.
pub fn revoke(store: &rdb::store::Store, id: &str, now: i64) -> Result<bool> {
    let key = token_key(id);
    let Some(mut row) = kv::get_json::<TokenRow>(store, &key)? else {
        return Ok(false);
    };
    match super::find_by_name(store, &row.token.user_name)? {
        Some(user) if user.role != Role::Admin => {}
        _ => return Ok(false),
    }
    if row.token.revoked_at.is_none() {
        row.token.revoked_at = Some(now);
        let mut batch = rocksdb::WriteBatch::default();
        tx::put(&mut batch, &key, &row)?;
        tx::commit(store, batch)?;
    }
    Ok(true)
}

/// Stage the deletion of every token row of `user_name` plus its indexes —
/// the user-delete cascade.
pub(super) fn stage_delete_for_user(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    user_name: &str,
) -> Result<()> {
    for (index_key, owner) in tx::scan_prefix(store, &user_index_prefix(user_name))? {
        if let Some(row) = kv::get_json::<TokenRow>(store, &owner)? {
            index::remove(batch, &hash_index(&row.token_hash));
        }
        tx::del(batch, &owner);
        tx::del(batch, &index_key);
    }
    Ok(())
}
