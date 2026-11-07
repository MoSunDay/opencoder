//! Platform identity over the rdb KV engine: users (`user/<name>` rows) and
//! their independently managed token digests (see [`tokens`]).
//!
//! Only sha256 hex digests are stored; plaintext credentials never reach
//! this layer. `name` is the users' primary key (a plain row-key duplicate
//! check), and the role is stored in its wire string so rows stay readable
//! by any tool that understands the SQL schema.

pub mod tokens;

use anyhow::Result;
use opencoder_core::identity::{parse_role, Role};
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv;
use crate::rdb_store::tx;
use crate::users::{GuardedDelete, PlatformUser};

pub fn user_key(name: &str) -> Vec<u8> {
    format!("user/{name}").into_bytes()
}

/// One `platform_users` row (role kept in its wire string).
#[derive(Clone, Serialize, Deserialize)]
struct UserRow {
    name: String,
    role: String,
    created_at: i64,
}

impl UserRow {
    fn into_platform_user(self) -> Result<PlatformUser> {
        Ok(PlatformUser {
            role: parse_role(&self.role).ok_or_else(|| {
                anyhow::anyhow!("platform user carries unknown role {:?}", self.role)
            })?,
            name: self.name,
            created_at: self.created_at,
        })
    }
}

/// Resolve a token digest to its live user (bearer-auth lookup; revoked or
/// expired tokens match nothing).
pub fn find_by_token_hash(
    store: &rdb::store::Store,
    token_hash: &str,
) -> Result<Option<PlatformUser>> {
    tokens::find_user_by_hash(store, token_hash)
}

pub fn find_by_name(store: &rdb::store::Store, name: &str) -> Result<Option<PlatformUser>> {
    kv::get_json::<UserRow>(store, &user_key(name))?
        .map(UserRow::into_platform_user)
        .transpose()
}

/// Every user, oldest first (`created_at, name`).
pub fn list_users(store: &rdb::store::Store) -> Result<Vec<PlatformUser>> {
    let mut users = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"user/")? {
        users.push(kv::decode::<UserRow>(&value)?.into_platform_user()?);
    }
    users.sort_by(|a, b| (&a.created_at, &a.name).cmp(&(&b.created_at, &b.name)));
    Ok(users)
}

/// Number of admin rows — the input of the last-admin guard.
pub fn count_admins(store: &rdb::store::Store) -> Result<i64> {
    let mut admins = 0i64;
    for (_, value) in tx::scan_prefix(store, b"user/")? {
        if kv::decode::<UserRow>(&value)?.role == Role::Admin.as_str() {
            admins += 1;
        }
    }
    Ok(admins)
}

/// Insert a user; a non-empty `token_hash` also creates the user's initial
/// token row in the same batch, so a hash conflict rolls the user back too
/// (UNIQUE(name) and UNIQUE(token_hash) both surface as constraint errors).
pub fn create_user(
    store: &rdb::store::Store,
    name: &str,
    token_hash: &str,
    role: Role,
    created_at: i64,
) -> Result<PlatformUser> {
    let key = user_key(name);
    if kv::get_json::<UserRow>(store, &key)?.is_some() {
        anyhow::bail!("insert platform user: UNIQUE constraint failed: platform_users.name");
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(
        &mut batch,
        &key,
        &UserRow {
            name: name.to_string(),
            role: role.as_str().to_string(),
            created_at,
        },
    )?;
    if !token_hash.is_empty() {
        tokens::stage_create(
            store,
            &mut batch,
            &crate::AccessToken {
                id: format!("user:{name}"),
                user_name: name.to_string(),
                name: tokens::INITIAL_TOKEN_LABEL.to_string(),
                created_at,
                expires_at: None,
                revoked_at: None,
            },
            token_hash,
        )?;
    }
    tx::commit(store, batch)?;
    find_by_name(store, name)?
        .ok_or_else(|| anyhow::anyhow!("create platform user: row for {name:?} vanished"))
}

/// Delete by name (the user's token rows cascade in the same batch);
/// `Ok(false)` when the user does not exist.
pub fn delete_user(store: &rdb::store::Store, name: &str) -> Result<bool> {
    if kv::get_json::<UserRow>(store, &user_key(name))?.is_none() {
        return Ok(false);
    }
    let mut batch = rocksdb::WriteBatch::default();
    crate::rdb_store::projects::people::stage_user_delete(store, &mut batch, name)?;
    tx::del(&mut batch, &user_key(name));
    tokens::stage_delete_for_user(store, &mut batch, name)?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Guarded delete: an admin row survives while it is the only one. The
/// count check and the delete run under the engine's single write lease, so
/// two racing deletions cannot both pass the count-then-act check.
pub fn delete_guarding_last_admin(store: &rdb::store::Store, name: &str) -> Result<GuardedDelete> {
    let Some(row) = kv::get_json::<UserRow>(store, &user_key(name))? else {
        return Ok(GuardedDelete::Missing);
    };
    if row.role == Role::Admin.as_str() && count_admins(store)? <= 1 {
        return Ok(GuardedDelete::LastAdmin);
    }
    delete_user(store, name)?;
    Ok(GuardedDelete::Deleted)
}

/// Re-point a user's initial token at a new digest; `Ok(false)` when the
/// user (or their initial token row) does not exist.
pub fn update_token_hash(store: &rdb::store::Store, name: &str, token_hash: &str) -> Result<bool> {
    tokens::update_hash(store, name, token_hash)
}

/// Change a non-admin's role. The startup admin credential is fixed, so
/// both writing `admin` onto a row and rewriting an existing admin are
/// rejected/no-op respectively.
pub fn update_role(store: &rdb::store::Store, name: &str, role: Role) -> Result<bool> {
    anyhow::ensure!(role != Role::Admin, "admin uses the startup credential");
    let key = user_key(name);
    let Some(mut row) = kv::get_json::<UserRow>(store, &key)? else {
        return Ok(false);
    };
    if row.role == Role::Admin.as_str() {
        return Ok(false);
    }
    row.role = role.as_str().to_string();
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &row)?;
    tx::commit(store, batch)?;
    Ok(true)
}
