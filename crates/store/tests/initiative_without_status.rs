//! Migration and progress checks use only disposable databases.
use opencoder_store::{LibsqlStore, ProjectStore};
use serde_json::json;

#[tokio::test]
async fn v37_removes_only_initiative_status_and_preserves_todo_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.db");
    let before;
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();
        conn.execute_batch(
            "ALTER TABLE project_initiatives ADD COLUMN status TEXT NOT NULL DEFAULT 'planned';
             INSERT INTO platform_users VALUES ('alice','viewer',1);
             INSERT INTO platform_tokens(id,user_name,name,token_hash,created_at)
               VALUES ('token','alice','fixture','unchanged-hash',1);
             UPDATE schema_version SET version=37;",
        )
        .await
        .unwrap();
        store
            .create_goal(
                &serde_json::from_value(json!({
                    "id":"p", "title":"Project", "status":"active", "sort":0,
                    "created_at":1,"updated_at":2
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        for (sort, status) in ["planned", "in_progress", "done"].iter().enumerate() {
            let id = format!("i{sort}");
            store
                .create_initiative(
                    &serde_json::from_value(json!({
                        "id":id,"goal_id":"p","title":id,"detail_md":"keep description",
                        "sort":sort,"created_at":1,"updated_at":2,
                        "current_summary":"keep summary","summary_updated_at":2,
                        "owner_user_name":"alice","follower_user_names":["alice"]
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            conn.execute(
                "UPDATE project_initiatives SET status=? WHERE id=?",
                [*status, &id],
            )
            .await
            .unwrap();
            store.create_todo(&serde_json::from_value(json!({
                "id":format!("t{sort}"),"initiative_id":id,"title":"Todo","draft":"keep task",
                "status":"planned","board_status":if sort == 0 {"done"} else {"todo"},
                "agent":"act","created_at":1,"updated_at":2
            })).unwrap()).await.unwrap();
        }
        conn.execute_batch(
            "INSERT INTO project_tags VALUES('tag','global','','keep tag');
            INSERT INTO project_todo_tags VALUES('t0','tag');",
        )
        .await
        .unwrap();
        before = opencoder_store::project::overview::load(&store)
            .await
            .unwrap();
    }
    for _ in 0..2 {
        let store = LibsqlStore::open(&path).await.unwrap();
        let conn = store.conn().await.unwrap();
        assert!(conn
            .query(
                "SELECT name FROM pragma_table_info('project_initiatives') WHERE name='status'",
                ()
            )
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .is_none());
        let after = opencoder_store::project::overview::load(&store)
            .await
            .unwrap();
        assert_eq!(
            after, before,
            "all retained project data survives migration and reopen"
        );
        for item in after["goals"][0]["initiatives"].as_array().unwrap() {
            assert!(item.get("status").is_none());
        }
        assert_eq!(
            store.get_todo("t0").await.unwrap().unwrap().board_status,
            "done"
        );
        assert_eq!(
            store.get_todo("t2").await.unwrap().unwrap().board_status,
            "todo"
        );
        assert_eq!(after["goals"][0]["progress"]["done"], 1);
        assert_eq!(after["goals"][0]["progress"]["total"], 3);
        let row = conn
            .query(
                "SELECT token_hash,created_at FROM platform_tokens WHERE id='token'",
                (),
            )
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.get::<String>(0).unwrap(), "unchanged-hash");
        assert_eq!(row.get::<i64>(1).unwrap(), 1);
    }
}

#[tokio::test]
async fn failed_status_migration_rolls_back_schema_and_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.db");
    {
        let store = LibsqlStore::open(&path).await.unwrap();
        store
            .conn()
            .await
            .unwrap()
            .execute_batch(
                "ALTER TABLE project_initiatives ADD COLUMN status TEXT NOT NULL DEFAULT 'planned';
             CREATE INDEX invalid_status_dependency ON project_initiatives(status);
             UPDATE schema_version SET version=37;",
            )
            .await
            .unwrap();
    }
    assert!(LibsqlStore::open(&path).await.is_err());
    let db = libsql::Builder::new_local(&path).build().await.unwrap();
    let conn = db.connect().unwrap();
    assert_eq!(
        conn.query("SELECT version FROM schema_version", ())
            .await
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap()
            .get::<i64>(0)
            .unwrap(),
        37
    );
    assert!(conn
        .query("SELECT status FROM project_initiatives", ())
        .await
        .is_ok());
    conn.execute("DROP INDEX invalid_status_dependency", ())
        .await
        .unwrap();
    assert!(LibsqlStore::open(&path).await.is_ok());
}
