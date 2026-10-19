use opencoder_database::{maintenance, Database};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

#[tokio::test]
async fn new_connections_preserve_an_existing_file_reader() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("readers.db");
    let database = Database::open(&path).await.unwrap();
    database
        .write(|connection| async move {
            connection
                .execute_batch(
                    "CREATE TABLE items(id INTEGER PRIMARY KEY); INSERT INTO items VALUES(1);",
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    drop(database);
    maintenance::checkpoint(path.clone()).await.unwrap();
    let database = Database::open(&path).await.unwrap();
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let first = database.clone();
    let notify = entered.clone();
    let proceed = resume.clone();
    let reader = tokio::spawn(async move {
        first
            .read(move |connection| async move {
                let mut rows = connection.query("SELECT id FROM items", ()).await?;
                assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, 1);
                drop(rows);
                notify.notify_one();
                proceed.notified().await;
                Ok(())
            })
            .await
    });
    entered.notified().await;
    let parallel = tokio::time::timeout(
        Duration::from_secs(5),
        database.read(|connection| async move {
            let mut rows = connection.query("SELECT count(*) FROM items", ()).await?;
            Ok(rows.next().await?.unwrap().get::<i64>(0)?)
        }),
    )
    .await;
    resume.notify_one();
    let existing = reader.await.unwrap();
    assert_eq!(parallel.unwrap().unwrap(), 1);
    existing.unwrap();
}
