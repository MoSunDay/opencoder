//! Owner/follower assignments, embedded in each project record row.
//!
//! The SQL backend keeps a separate `project_people` table validated
//! against `platform_users`; the rdb layout stores the assignment inside
//! the record JSON (`ProjectPeople` is `#[serde(flatten)]`), so there is no
//! separate row family to cascade. The validation is identical: every name
//! referenced by a patch must belong to an existing platform user, else the
//! whole mutation (row write, tag reconciliation, …) aborts with
//! `InvalidProjectUser`.

use anyhow::Result;

use crate::project::people::{InvalidProjectUser, ProjectPeople, ProjectPeoplePatch};
use crate::rdb_store::identity;

/// Does the platform identity module know this user? (`platform_users`
/// check in the SQL backend; the identity module owns the row content.)
pub fn user_exists(store: &rdb::store::Store, name: &str) -> Result<bool> {
    Ok(identity::find_by_name(store, name)?.is_some())
}

/// Names an existing platform user must back before any write is staged.
pub fn validate(store: &rdb::store::Store, patch: &ProjectPeoplePatch) -> Result<()> {
    let owner = patch.owner_user_name.as_ref().and_then(|v| v.as_deref());
    for name in owner.into_iter().chain(
        patch
            .follower_user_names
            .iter()
            .flatten()
            .map(String::as_str),
    ) {
        anyhow::ensure!(user_exists(store, name)?, InvalidProjectUser(name.into()));
    }
    Ok(())
}

/// Canonical form for stored rows: followers deduplicated and ordered —
/// the SQL `BTreeSet` INSERT loop in `project_people`.
pub fn normalize(people: &ProjectPeople) -> ProjectPeople {
    let mut normalized = people.clone();
    apply(
        &mut normalized,
        &ProjectPeoplePatch {
            owner_user_name: Some(people.owner_user_name.clone()),
            follower_user_names: Some(people.follower_user_names.clone()),
        },
    );
    normalized
}

/// Apply a patch onto a record's people: `Some(None)` clears the owner,
/// `Some(vec)` replaces the follower set (deduplicated, ordered).
pub fn apply(people: &mut ProjectPeople, patch: &ProjectPeoplePatch) {
    if let Some(owner) = patch.owner_user_name.clone() {
        people.owner_user_name = owner;
    }
    if let Some(followers) = patch.follower_user_names.clone() {
        people.follower_user_names = followers
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
    }
}

/// Strip `user` from every embedded people list before the user row is
/// deleted (the SQL schema's `ON DELETE CASCADE` on `project_people`).
/// Objects themselves are untouched.
pub fn stage_user_delete(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    user: &str,
) -> Result<()> {
    use crate::project_types::{ProjectGoalRecord, ProjectInitiativeRecord, ProjectTodoRecord};
    use crate::rdb_store::projects::keys;
    use crate::rdb_store::{kv, tx};

    fn strip(people: &mut ProjectPeople, user: &str) -> bool {
        let mut changed = people.owner_user_name.as_deref() == Some(user);
        if changed {
            people.owner_user_name = None;
        }
        let before = people.follower_user_names.len();
        people.follower_user_names.retain(|name| name != user);
        changed |= people.follower_user_names.len() != before;
        changed
    }

    for (_, value) in tx::scan_prefix(store, b"pgl/")? {
        let mut rec: ProjectGoalRecord = kv::decode(&value)?;
        if strip(&mut rec.people, user) {
            tx::put(batch, &keys::goal_key(&rec.id), &rec)?;
        }
    }
    for (_, value) in tx::scan_prefix(store, b"pin/")? {
        let mut rec: ProjectInitiativeRecord = kv::decode(&value)?;
        if strip(&mut rec.people, user) {
            tx::put(batch, &keys::initiative_key(&rec.id), &rec)?;
        }
    }
    for (_, value) in tx::scan_prefix(store, &keys::todo_prefix())? {
        let mut rec: ProjectTodoRecord = kv::decode(&value)?;
        if strip(&mut rec.people, user) {
            tx::put(batch, &keys::todo_key(&rec.id), &rec)?;
        }
    }
    Ok(())
}
