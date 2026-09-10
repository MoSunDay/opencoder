//! Platform users (`platform_users`): name UNIQUE, token_hash UNIQUE, role.
//! Only sha256 digests are stored (see `opencoder_core::token_hash`); tokens
//! are returned exactly once at creation time by the control API.

use anyhow::{Context, Result};
use libsql::{Connection, params};

use crate::users::{GuardedDelete, PlatformUser};
use opencoder_core::identity::Role;

const USER_COLS: &str = "name, role, created_at";

fn role_from_wire(value: &str) -> Result<Role> {
    opencoder_core::parse_role(value)
        .ok_or_else(|| anyhow::anyhow!("platform user carries unknown role {value:?}"))
}

fn row_to_user(row: &libsql::Row) -> Result<PlatformUser> {
    Ok(PlatformUser {
        name: row.get(0)?,
        role: role_from_wire(&row.get::<String>(1)?)?,
        created_at: row.get(2)?,
    })
}

/// Resolve a token digest to its user (bearer authentication lookup).
pub async fn find_by_token_hash(
    conn: &Connection,
    token_hash: &str,
) -> Result<Option<PlatformUser>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {USER_COLS} FROM platform_users WHERE token_hash = ?1"
        ))
        .await?;
    let mut rows = stmt.query(params![token_hash]).await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub async fn find_by_name(conn: &Connection, name: &str) -> Result<Option<PlatformUser>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {USER_COLS} FROM platform_users WHERE name = ?1"
        ))
        .await?;
    let mut rows = stmt.query(params![name]).await?;
    match rows.next().await? {
        Some(row) => Ok(Some(row_to_user(&row)?)),
        None => Ok(None),
    }
}

pub async fn list(conn: &Connection) -> Result<Vec<PlatformUser>> {
    let stmt = conn
        .prepare(&format!(
            "SELECT {USER_COLS} FROM platform_users ORDER BY created_at ASC, name ASC"
        ))
        .await?;
    let mut rows = stmt.query(()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row_to_user(&row)?);
    }
    Ok(out)
}

pub async fn count_admins(conn: &Connection) -> Result<i64> {
    let stmt = conn
        .prepare("SELECT COUNT(*) FROM platform_users WHERE role = 'admin'")
        .await?;
    let mut rows = stmt.query(()).await?;
    Ok(rows
        .next()
        .await?
        .map(|r| r.get(0))
        .transpose()?
        .unwrap_or(0))
}

/// Insert a user; UNIQUE(name) / UNIQUE(token_hash) violations surface as
/// libsql `SqliteFailure` errors the API layer maps to 409.
pub async fn create(
    conn: &Connection,
    name: &str,
    token_hash: &str,
    role: Role,
    created_at: i64,
) -> Result<PlatformUser> {
    conn.execute(
        "INSERT INTO platform_users (name, token_hash, role, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, token_hash, role.as_str(), created_at],
    )
    .await
    .context("create platform user")?;
    find_by_name(conn, name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("create platform user: row for {name:?} vanished"))
}

/// Delete by name. Returns false when the user does not exist.
pub async fn delete(conn: &Connection, name: &str) -> Result<bool> {
    let changed = conn
        .execute("DELETE FROM platform_users WHERE name = ?1", params![name])
        .await
        .context("delete platform user")?;
    Ok(changed > 0)
}

/// Atomic delete guarded by the last-admin rule. The sub-select runs in the
/// same statement as the delete, so concurrent deletions serialize on the
/// row write and can never remove the final admin. `Missing`/`LastAdmin`
/// are resolved after the fact purely to shape the API response.
pub async fn delete_guarding_last_admin(conn: &Connection, name: &str) -> Result<GuardedDelete> {
    let changed = conn
        .execute(
            "DELETE FROM platform_users WHERE name = ?1 \
             AND (role != 'admin' OR (SELECT COUNT(*) FROM platform_users WHERE role = 'admin') > 1)",
            params![name],
        )
        .await
        .context("delete platform user (last-admin guarded)")?;
    if changed > 0 {
        return Ok(GuardedDelete::Deleted);
    }
    match find_by_name(conn, name).await? {
        None => Ok(GuardedDelete::Missing),
        Some(_) => Ok(GuardedDelete::LastAdmin),
    }
}

/// Re-point a user's credential at a new token digest (seed rotation).
/// Returns false when no row carries that name; a UNIQUE(token_hash)
/// collision with another user surfaces as an error.
pub async fn update_token_hash(conn: &Connection, name: &str, token_hash: &str) -> Result<bool> {
    let changed = conn
        .execute(
            "UPDATE platform_users SET token_hash = ?2 WHERE name = ?1",
            params![name, token_hash],
        )
        .await
        .context("rotate platform user token")?;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Connection {
        let store = crate::LibsqlStore::open_memory().await.unwrap();
        store.conn().await.unwrap()
    }

    #[tokio::test]
    async fn crud_roundtrip_and_unique_constraints() {
        let conn = db().await;
        let created = create(&conn, "alice", &"h1".repeat(8), Role::User, 10)
            .await
            .unwrap();
        assert_eq!(created.name, "alice");
        assert_eq!(created.role, Role::User);

        // Same name conflicts.
        assert!(
            create(&conn, "alice", &"h2".repeat(8), Role::Root, 11)
                .await
                .is_err()
        );
        // Same token hash conflicts (different name).
        assert!(
            create(&conn, "bob", &"h1".repeat(8), Role::User, 12)
                .await
                .is_err()
        );
        // Distinct user is fine.
        create(&conn, "bob", &"h3".repeat(8), Role::Admin, 13)
            .await
            .unwrap();

        assert_eq!(
            find_by_token_hash(&conn, &"h1".repeat(8))
                .await
                .unwrap()
                .map(|u| u.name),
            Some("alice".into())
        );
        assert_eq!(
            find_by_token_hash(&conn, &"nope".repeat(8)).await.unwrap(),
            None
        );
        assert_eq!(
            list(&conn)
                .await
                .unwrap()
                .iter()
                .map(|u| u.name.clone())
                .collect::<Vec<_>>(),
            vec!["alice".to_string(), "bob".to_string()]
        );
        assert_eq!(count_admins(&conn).await.unwrap(), 1);

        assert!(delete(&conn, "alice").await.unwrap());
        assert!(!delete(&conn, "alice").await.unwrap());
        assert_eq!(count_admins(&conn).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn guarded_delete_never_removes_the_last_admin() {
        let conn = db().await;
        create(&conn, "solo", &"h1".repeat(8), Role::Admin, 10)
            .await
            .unwrap();
        // The only admin is refused…
        assert_eq!(
            delete_guarding_last_admin(&conn, "solo").await.unwrap(),
            GuardedDelete::LastAdmin
        );
        assert_eq!(count_admins(&conn).await.unwrap(), 1);
        // …unknown names report Missing…
        assert_eq!(
            delete_guarding_last_admin(&conn, "ghost").await.unwrap(),
            GuardedDelete::Missing
        );
        // …non-admins always delete…
        create(&conn, "plain", &"h2".repeat(8), Role::User, 11)
            .await
            .unwrap();
        assert_eq!(
            delete_guarding_last_admin(&conn, "plain").await.unwrap(),
            GuardedDelete::Deleted
        );
        // …and a second admin unlocks removal — the guard reads the count
        // inside the same statement, so sequential deletes of both admins
        // stop at the survivor exactly like concurrent ones would.
        create(&conn, "peer", &"h3".repeat(8), Role::Admin, 12)
            .await
            .unwrap();
        assert_eq!(
            delete_guarding_last_admin(&conn, "solo").await.unwrap(),
            GuardedDelete::Deleted
        );
        assert_eq!(
            delete_guarding_last_admin(&conn, "peer").await.unwrap(),
            GuardedDelete::LastAdmin
        );
    }

    #[tokio::test]
    async fn token_hash_rotation_repoints_and_keeps_uniqueness() {
        let conn = db().await;
        create(&conn, "admin", &"h1".repeat(8), Role::Admin, 10)
            .await
            .unwrap();
        assert!(
            update_token_hash(&conn, "admin", &"h2".repeat(8))
                .await
                .unwrap()
        );
        assert_eq!(
            find_by_token_hash(&conn, &"h1".repeat(8)).await.unwrap(),
            None
        );
        assert_eq!(
            find_by_token_hash(&conn, &"h2".repeat(8))
                .await
                .unwrap()
                .map(|u| u.name),
            Some("admin".into())
        );
        // Unknown names are a no-op; another user's digest is rejected.
        assert!(
            !update_token_hash(&conn, "ghost", &"h3".repeat(8))
                .await
                .unwrap()
        );
        create(&conn, "other", &"h4".repeat(8), Role::User, 11)
            .await
            .unwrap();
        assert!(
            update_token_hash(&conn, "other", &"h2".repeat(8))
                .await
                .is_err(),
            "rotating onto an existing digest must not be silently allowed"
        );
    }
}
