//! One relation table for human owners and followers; no credential changes.
use anyhow::Result;
use turso::Connection;

pub(super) async fn initialize(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS project_people (
        object_kind TEXT NOT NULL CHECK(object_kind IN ('project','initiative','todo')),
        object_id TEXT NOT NULL,
        user_name TEXT NOT NULL REFERENCES platform_users(name) ON DELETE CASCADE,
        relation TEXT NOT NULL CHECK(relation IN ('owner','follower')),
        PRIMARY KEY(object_kind,object_id,relation,user_name))",
        (),
    )
    .await?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS project_people_owner
        ON project_people(object_kind,object_id) WHERE relation='owner'",
        (),
    )
    .await?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS project_people_user ON project_people(user_name)",
        (),
    )
    .await?;
    for (kind, table) in [
        ("project", "project_goals"),
        ("initiative", "project_initiatives"),
        ("todo", "project_todos"),
    ] {
        conn.execute(
            &format!(
                "CREATE TRIGGER IF NOT EXISTS project_people_delete_{kind}
            AFTER DELETE ON {table} BEGIN
            DELETE FROM project_people WHERE object_kind='{kind}' AND object_id=OLD.id; END"
            ),
            (),
        )
        .await?;
    }
    Ok(())
}
