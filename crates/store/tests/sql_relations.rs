//! Each test creates its own database; existing databases and records are untouched.
#[cfg(any(feature = "mysql", feature = "starrocks"))]
mod gated {
    use opencoder_core::{StorageBackend, StorageConfig};
    use opencoder_store::{
        sql_store, ProjectGoalRecord, ProjectGoalStatus, ProjectMilestonePatch, ProjectTodoPatch,
    };
    use sqlx::{mysql::MySqlConnectOptions, MySqlPool, Row};

    async fn contract(variable: &str, starrocks: bool) {
        let Ok(dsn) = std::env::var(variable) else {
            eprintln!("{variable} not set: live SQL test skipped");
            return;
        };
        let mut options: MySqlConnectOptions = dsn.parse().unwrap();
        if starrocks {
            options = options
                .pipes_as_concat(false)
                .no_engine_substitution(false)
                .timezone(None)
                .set_names(false);
        }
        let root = MySqlPool::connect_with(options.clone()).await.unwrap();
        let db = format!(
            "oc_relations_{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        sqlx::raw_sql(&format!("CREATE DATABASE {db}"))
            .execute(&root)
            .await
            .unwrap();
        let pool = MySqlPool::connect_with(options.database(&db))
            .await
            .unwrap();
        let suffix = if starrocks {
            ") PRIMARY KEY(id) DISTRIBUTED BY HASH(id) BUCKETS 1"
        } else {
            ", PRIMARY KEY(id)) ENGINE=InnoDB"
        };
        sqlx::raw_sql(&format!(
            "CREATE TABLE project_milestones (
            id VARCHAR(64) NOT NULL,goal_id VARCHAR(64) NOT NULL,title VARCHAR(512) NOT NULL,
            detail_md VARCHAR(2048) NULL,status VARCHAR(32) NOT NULL,sort_key BIGINT NOT NULL,
            created_at BIGINT NOT NULL,updated_at BIGINT NOT NULL{suffix}"
        ))
        .execute(&pool)
        .await
        .unwrap();
        sql_store::ddl::apply(&pool, starrocks).await.unwrap();
        sqlx::raw_sql("INSERT INTO project_milestones VALUES('old-m','old-g','existing','retained text','planned',0,1,2)")
            .execute(&pool).await.unwrap();
        sqlx::raw_sql("INSERT INTO project_todos (id,milestone_id,title,draft,plan_md,status,agent,created_at,updated_at)
            VALUES('old-t',NULL,'old task','retained draft','retained plan','planned','act',3,4)")
            .execute(&pool).await.unwrap();
        let mut url = url_dsn(&dsn, &db);
        let config = StorageConfig {
            backend: if starrocks {
                StorageBackend::Starrocks
            } else {
                StorageBackend::Mysql
            },
            mysql: (!starrocks).then(|| url.clone()),
            starrocks: starrocks.then(|| std::mem::take(&mut url)),
        };
        let store = sql_store::open(&config).await.unwrap();
        let old = store.get_todo("old-t").await.unwrap().unwrap();
        assert_eq!(old.draft, "retained draft");
        assert_eq!(old.plan_md.as_deref(), Some("retained plan"));
        assert_eq!(old.updated_at, 4);
        assert!(old.milestone_id.is_some());
        let all = store.list_milestones(None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.iter()
                .find(|m| m.id == "old-m")
                .unwrap()
                .detail_md
                .as_deref(),
            Some("retained text")
        );
        let standalone = all.iter().find(|m| m.goal_id.is_none()).unwrap();
        assert!(store
            .delete_milestone(&standalone.id)
            .await
            .unwrap_err()
            .is::<opencoder_store::project::MilestoneNotEmpty>());
        store
            .create_goal(&ProjectGoalRecord {
                id: "old-g".into(),
                title: "project to detach".into(),
                detail_md: None,
                status: ProjectGoalStatus::Active,
                sort: 0,
                created_at: 1,
                updated_at: 2,
            })
            .await
            .unwrap();
        store
            .patch_milestone(
                &standalone.id,
                &ProjectMilestonePatch {
                    goal_id: Some(Some("old-g".into())),
                    ..Default::default()
                },
                5,
            )
            .await
            .unwrap();
        assert!(store.delete_goal("old-g").await.unwrap());
        assert!(store
            .list_milestones(None)
            .await
            .unwrap()
            .iter()
            .all(|m| m.goal_id.is_none()));
        assert_eq!(
            serde_json::to_value(store.get_todo("old-t").await.unwrap().unwrap()).unwrap(),
            serde_json::to_value(&old).unwrap()
        );
        store
            .patch_milestone(
                "old-m",
                &ProjectMilestonePatch {
                    goal_id: Some(None),
                    ..Default::default()
                },
                5,
            )
            .await
            .unwrap();
        store
            .patch_todo(
                "old-t",
                &ProjectTodoPatch {
                    milestone_id: Some(None),
                    ..Default::default()
                },
                6,
            )
            .await
            .unwrap();
        drop(store);
        let store = sql_store::open(&config).await.unwrap();
        assert!(store
            .get_todo("old-t")
            .await
            .unwrap()
            .unwrap()
            .milestone_id
            .is_none());
        let columns = sqlx::raw_sql("SELECT IS_NULLABLE FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name='project_milestones' AND column_name='goal_id'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(columns[0].try_get::<String, _>(0).unwrap(), "YES");
    }

    fn url_dsn(dsn: &str, database: &str) -> String {
        let (base, query) = dsn.split_once('?').map_or((dsn, ""), |(a, b)| (a, b));
        let authority = base.find("://").map(|i| i + 3).unwrap();
        let end = base[authority..]
            .find('/')
            .map(|i| authority + i)
            .unwrap_or(base.len());
        format!(
            "{}/{database}{}",
            &base[..end],
            if query.is_empty() {
                String::new()
            } else {
                format!("?{query}")
            }
        )
    }

    #[tokio::test]
    async fn mysql_relations_upgrade() {
        contract("OC_TEST_MYSQL_DSN", false).await;
    }
    #[tokio::test]
    async fn starrocks_relations_upgrade() {
        contract("OC_TEST_STARROCKS_DSN", true).await;
    }
}
