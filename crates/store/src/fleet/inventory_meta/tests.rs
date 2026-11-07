use super::*;
use crate::fleet::FleetConnection;
use opencoder_database::Database;

#[tokio::test]
async fn sleep_prefix_tracks_new_and_legacy_writes_without_schema_changes() {
    let store = FleetStore::open_memory().await.unwrap();
    let body = serde_json::json!({"indexes":[],"payload":"x".repeat(1024*1024)});
    assert_eq!(
        store.runtime_sleep_stamp("runtime").await.unwrap(),
        (String::new(), false)
    );
    store
        .put_definition("runtime_sleep", "runtime", &body)
        .await
        .unwrap();
    let first = store.runtime_sleep_stamp("runtime").await.unwrap();
    assert!(first.1);
    assert_eq!(first, store.runtime_sleep_stamp("runtime").await.unwrap());
    assert_eq!(
        store.definition("runtime_sleep", "runtime").await.unwrap(),
        Some(body.clone())
    );
    store
        .put_definition("runtime_sleep", "runtime", &body)
        .await
        .unwrap();
    assert_ne!(first, store.runtime_sleep_stamp("runtime").await.unwrap());
    let legacy = body.clone();
    store
        .db
        .write(move |conn| async move {
            conn.execute(
                "UPDATE fleet_definitions SET body=?1 WHERE kind='runtime_sleep' AND id='runtime'",
                [serde_json::to_string(&legacy)?],
            )
            .await?;
            Ok(())
        })
        .await
        .unwrap();
    let stamped = store.runtime_sleep_stamp("runtime").await.unwrap();
    assert!(stamped.1);
    assert_ne!(first, stamped);
    assert_eq!(stamped, store.runtime_sleep_stamp("runtime").await.unwrap());
    assert_eq!(
        store.definition("runtime_sleep", "runtime").await.unwrap(),
        Some(body)
    );
    store
        .put_definition("runtime_sleep", "runtime", &Value::Null)
        .await
        .unwrap();
    assert_eq!(
        store.runtime_sleep_stamp("runtime").await.unwrap(),
        (String::new(), false)
    );
    store
        .delete_definition("runtime_sleep", "runtime")
        .await
        .unwrap();
    assert_eq!(
        store.runtime_sleep_stamp("runtime").await.unwrap(),
        (String::new(), false)
    );
}

#[tokio::test]
async fn shared_database_upgrade_preserves_the_running_old_connections() {
    const CHILD: &str = "OPENCODER_FLEET_UPGRADE_TEST";
    if let Some(path) = std::env::var_os(CHILD) {
        let store = FleetStore::open(std::path::Path::new(&path)).await.unwrap();
        assert!(store.runtime_sleep_stamp("legacy").await.unwrap().1);
        std::fs::write(std::path::Path::new(&path).with_extension("opened"), b"opened")
            .unwrap();
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("host.db");
    let old = Database::open(&path).await.unwrap();
    // Exactly the established Fleet schema, without the attempted projection DDL.
    old.write(|conn| async move {
        crate::fleet::schema::initialize(&conn).await?;
        crate::fleet::brain::initialize(&conn).await?;
        crate::fleet::handoff::initialize(&conn).await?;
        conn.execute(
            "INSERT INTO fleet_definitions VALUES('runtime_sleep','legacy','{\"indexes\":[]}')",
            (),
        )
        .await?;
        FleetConnection { conn }.initialize_capacity(2).await?;
        Ok(())
    })
    .await
    .unwrap();
    let before = old
        .read(|conn| async move {
            let mut rows = conn.query("PRAGMA schema_version", ()).await?;
            let version = rows.next().await?.unwrap().get::<i64>(0)?;
            drop(rows);
            assert_eq!(FleetConnection { conn }.capacity().await?.max_runs, 2);
            Ok(version)
        })
        .await
        .unwrap();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["fleet::inventory_meta::tests::shared_database_upgrade_preserves_the_running_old_connections", "--exact", "--nocapture"])
            .env(CHILD,path).status().unwrap()
    }).await.unwrap();
    assert!(status.success());
    assert!(root.path().join("host.opened").is_file());
    for _ in 0..8 {
        old.read(move |conn| async move {
            let mut rows = conn.query("PRAGMA schema_version", ()).await?;
            assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, before);
            drop(rows);
            assert_eq!(FleetConnection { conn }.capacity().await?.max_runs, 2);
            Ok(())
        })
        .await
        .unwrap();
    }
}
