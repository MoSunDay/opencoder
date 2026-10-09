//! Historical catalogs plus the deployment controller's project restore.
use opencoder_store::LibsqlStore;
use std::{path::Path, process::Command};

async fn scalar(conn: &libsql::Connection, sql: &str) -> String {
    conn.query(sql, ())
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap()
        .get::<String>(0)
        .unwrap()
}

fn python(root: &Path, source: &Path, live: &Path, script: &str) {
    let result = Command::new("python3")
        .args(["-I", "-c", script])
        .arg(root.join("scripts/platform"))
        .arg(source)
        .arg(live)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
async fn v31_migration_restarts_and_restores_old_project_writes_without_auth_changes() {
    migrate_and_restore(31).await;
}

#[tokio::test]
async fn legacy_v32_catalog_migrates_and_restores_old_writes_without_auth_changes() {
    migrate_and_restore(32).await;
}

async fn migrate_and_restore(version: i64) {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("live.db");
    let saved = dir.path().join("saved.db");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    {
        let store = LibsqlStore::open(&live).await.unwrap();
        let conn = store.conn().await.unwrap();
        conn.execute_batch(
            "DROP TABLE project_tags; DROP TABLE project_todo_tags; DROP TABLE project_initiatives;
             ALTER TABLE project_todos RENAME COLUMN initiative_id TO milestone_id;
             CREATE TABLE project_milestones (
               id TEXT PRIMARY KEY,kind TEXT NOT NULL,goal_id TEXT,title TEXT NOT NULL,
               detail_md TEXT,status TEXT NOT NULL,sort_key INTEGER NOT NULL,
               created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
             INSERT INTO project_milestones VALUES ('initiative','initiative',NULL,'preserved',NULL,'planned',0,1,1);
             INSERT INTO project_milestones VALUES ('old-marker','milestone',NULL,'marker',NULL,'planned',0,1,1);
             INSERT INTO project_todos(id,milestone_id,title,draft,status,agent,created_at,updated_at)
               VALUES ('one','initiative','todo','draft','draft','act',1,1),
                      ('two','old-marker','todo','draft','draft','act',1,1);
             CREATE INDEX idx_project_todos_milestone ON project_todos(milestone_id);
             INSERT INTO platform_users VALUES ('fixture','viewer',1);
             INSERT INTO platform_tokens (id,user_name,name,token_hash,created_at) VALUES ('fixture','fixture','test','fixture-hash',1);
             UPDATE schema_version SET version=31;",
        ).await.unwrap();
        conn.execute("UPDATE schema_version SET version=?", [version])
            .await
            .unwrap();
    }
    python(root, &live, &saved, "import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from rolling.backup import database; database(Path(sys.argv[2]),Path(sys.argv[3]))");
    for _ in 0..2 {
        let store = LibsqlStore::open(&live).await.unwrap();
        let conn = store.conn().await.unwrap();
        assert_eq!(
            scalar(&conn, "SELECT CAST(version AS TEXT) FROM schema_version").await,
            "35"
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT title FROM project_initiatives WHERE id='initiative'"
            )
            .await,
            "preserved"
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT initiative_id FROM project_todos WHERE id='one'"
            )
            .await,
            "initiative"
        );
        assert_eq!(scalar(&conn, "SELECT CAST(count(*) AS TEXT) FROM project_todos WHERE id='two' AND initiative_id IS NULL").await, "1");
        assert_eq!(
            scalar(
                &conn,
                "SELECT token_hash FROM platform_tokens WHERE user_name='fixture'"
            )
            .await,
            "fixture-hash"
        );
    }
    python(root, &saved, &live, "import sys; from pathlib import Path; sys.path.insert(0,sys.argv[1]); from rolling.maintenance.archive import restore_projects; restore_projects(Path(sys.argv[2]),Path(sys.argv[3]))");
    // Old SQL reads/writes must work without reopening the current Store,
    // which would correctly migrate the restored database forward again.
    let db = libsql::Builder::new_local(&live).build().await.unwrap();
    let conn = db.connect().unwrap();
    assert_eq!(
        scalar(&conn, "SELECT CAST(version AS TEXT) FROM schema_version").await,
        version.to_string()
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT milestone_id FROM project_todos WHERE id='two'"
        )
        .await,
        "old-marker"
    );
    conn.execute(
        "UPDATE project_milestones SET title='old-write' WHERE id='initiative'",
        (),
    )
    .await
    .unwrap();
    conn.execute(
        "UPDATE project_todos SET milestone_id='initiative' WHERE id='two'",
        (),
    )
    .await
    .unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT title FROM project_milestones WHERE id='initiative'"
        )
        .await,
        "old-write"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT token_hash FROM platform_tokens WHERE user_name='fixture'"
        )
        .await,
        "fixture-hash"
    );
    assert_eq!(scalar(&conn, "PRAGMA integrity_check").await, "ok");
}
