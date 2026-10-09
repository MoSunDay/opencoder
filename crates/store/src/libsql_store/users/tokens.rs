use crate::AccessToken;
use anyhow::Result;
use libsql::{params, Connection};

pub async fn list(conn: &Connection) -> Result<Vec<AccessToken>> {
    let mut rows = conn.query("SELECT id,user_name,name,created_at,expires_at,revoked_at FROM platform_tokens ORDER BY created_at DESC,id", ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(AccessToken {
            id: row.get(0)?,
            user_name: row.get(1)?,
            name: row.get(2)?,
            created_at: row.get(3)?,
            expires_at: row.get(4)?,
            revoked_at: row.get(5)?,
        });
    }
    Ok(out)
}

pub async fn create(conn: &Connection, token: &AccessToken, hash: &str) -> Result<()> {
    conn.execute("INSERT INTO platform_tokens (id,user_name,name,token_hash,created_at,expires_at,revoked_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![token.id.clone(),token.user_name.clone(),token.name.clone(),hash,token.created_at,token.expires_at,token.revoked_at]).await?;
    Ok(())
}

pub async fn revoke(conn: &Connection, id: &str, now: i64) -> Result<bool> {
    Ok(conn.execute("UPDATE platform_tokens SET revoked_at = COALESCE(revoked_at,?2) WHERE id=?1 AND user_name IN (SELECT name FROM platform_users WHERE role != 'admin')", params![id,now]).await? > 0)
}
