//! Platform user records (name + role + token digest). The SQL lives in
//! `libsql_store::users`; this module owns the wire-shaped record type only.

use opencoder_core::identity::Role;
use serde::{Deserialize, Serialize};

/// One platform user. `token_hash` never leaves the store: API responses are
/// built from `PlatformUser` rows without the digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformUser {
    pub name: String,
    pub role: Role,
    pub created_at: i64,
}

/// Outcome of `Store::delete_user_guarding_last_admin`. The guard runs in
/// the same statement as the delete, so two concurrent admin deletions can
/// never both pass a count-then-act check and empty the table of admins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardedDelete {
    /// Row removed by this call.
    Deleted,
    /// No row carries that name (possibly removed concurrently).
    Missing,
    /// Row kept: it is the last admin.
    LastAdmin,
}
