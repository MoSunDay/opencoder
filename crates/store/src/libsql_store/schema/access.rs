//! Split user identity from credentials without rotating or expiring a digest.
use anyhow::Result;
use libsql::Connection;

pub(super) const USERS: &str = "CREATE TABLE IF NOT EXISTS platform_users (
    name TEXT PRIMARY KEY, role TEXT NOT NULL, created_at INTEGER NOT NULL)";
const TOKENS: &str = "CREATE TABLE IF NOT EXISTS platform_tokens (
    id TEXT PRIMARY KEY,
    user_name TEXT NOT NULL REFERENCES platform_users(name) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    revoked_at INTEGER)";

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    if super::column_exists(conn, "platform_users", "token_hash").await? {
        conn.execute(
            "ALTER TABLE platform_users RENAME TO platform_users_v33",
            (),
        )
        .await?;
        conn.execute(USERS, ()).await?;
        conn.execute(
            "INSERT INTO platform_users SELECT name, role, created_at FROM platform_users_v33",
            (),
        )
        .await?;
        conn.execute(TOKENS, ()).await?;
        conn.execute("INSERT INTO platform_tokens (id,user_name,name,token_hash,created_at)
            SELECT 'user:' || name, name, '初始 Token', token_hash, created_at FROM platform_users_v33", ()).await?;
        conn.execute("DROP TABLE platform_users_v33", ()).await?;
    } else {
        conn.execute(TOKENS, ()).await?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_platform_tokens_user ON platform_tokens(user_name)",
        (),
    )
    .await?;
    Ok(())
}
