use opencoder_database::{maintenance, Database};
use std::process::{Command, Stdio};

#[tokio::test]
async fn processes_share_committed_writes_and_online_backup() {
    if let Ok(path) = std::env::var("OPENCODER_TEST_DATABASE") {
        let child: i64 = std::env::var("OPENCODER_TEST_WRITER")
            .unwrap()
            .parse()
            .unwrap();
        let database = Database::open(path).await.unwrap();
        for sequence in 0..20 {
            database
                .write(move |connection| async move {
                    connection
                        .execute(
                            "INSERT INTO items VALUES(?1, ?2)",
                            [child * 100 + sequence, sequence],
                        )
                        .await?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.db");
    let database = Database::open(&path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute(
                    "CREATE TABLE items(id INTEGER PRIMARY KEY, value INTEGER)",
                    (),
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    let children = (0..4)
        .map(|child| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "processes_share_committed_writes_and_online_backup",
                    "--nocapture",
                ])
                .env("OPENCODER_TEST_DATABASE", &path)
                .env("OPENCODER_TEST_WRITER", child.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let copy = directory.path().join("snapshot.db");
    maintenance::backup(path.clone(), copy.clone())
        .await
        .unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let result = maintenance::query(
        path.clone(),
        "SELECT count(*),sum(value) FROM items".into(),
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(result["rows"], serde_json::json!([[80, 760]]));
    // A stopped checkpoint must produce a self-contained file for a rollback.
    drop(database);
    maintenance::checkpoint(path.clone()).await.unwrap();
    assert_eq!(
        std::fs::metadata(path.with_extension("db-wal"))
            .map(|m| m.len())
            .unwrap_or(0),
        0
    );
    assert!(
        maintenance::query(copy, "SELECT count(*) FROM items".into(), vec![])
            .await
            .is_ok()
    );
}
