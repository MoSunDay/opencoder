use opencoder_database::Database;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

async fn database() -> Database {
    let db = Database::open_memory().await.unwrap();
    db.write(|conn| async move {
        conn.execute_batch("CREATE TABLE values_test(id INTEGER PRIMARY KEY, value INTEGER); INSERT INTO values_test VALUES(1, 0);").await?;
        Ok(())
    }).await.unwrap();
    db
}

async fn value(db: &Database) -> i64 {
    db.read(|conn| async move {
        let mut rows = conn
            .query("SELECT value FROM values_test WHERE id=1", ())
            .await?;
        Ok(rows.next().await?.unwrap().get::<i64>(0)?)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn disconnected_writer_finishes_while_readers_keep_the_previous_snapshot() {
    let db = database().await;
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let (notify, proceed, writer) = (entered.clone(), resume.clone(), db.clone());
    let client = tokio::spawn(async move {
        writer
            .write(move |conn| async move {
                conn.execute_batch("BEGIN IMMEDIATE; UPDATE values_test SET value=1 WHERE id=1;")
                    .await?;
                notify.notify_one();
                proceed.notified().await;
                conn.execute("COMMIT", ()).await?;
                Ok(())
            })
            .await
    });
    entered.notified().await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), value(&db))
            .await
            .unwrap(),
        0
    );
    client.abort();
    resume.notify_one();
    db.write(|_| async { Ok(()) }).await.unwrap();
    assert_eq!(value(&db).await, 1);
}

#[tokio::test]
async fn reads_use_a_snapshot_and_failed_writes_do_not_poison_the_next_operation() {
    let db = database().await;
    let writer = db.clone();
    db.read(move |conn| async move {
        let mut rows = conn.query("SELECT value FROM values_test", ()).await?;
        assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, 0);
        drop(rows);
        writer
            .write(|conn| async move {
                conn.execute("UPDATE values_test SET value=2", ()).await?;
                Ok(())
            })
            .await?;
        let mut rows = conn.query("SELECT value FROM values_test", ()).await?;
        assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, 0);
        Ok(())
    })
    .await
    .unwrap();
    assert!(db
        .write::<_, _, ()>(|conn| async move {
            conn.execute_batch("BEGIN IMMEDIATE; UPDATE values_test SET value=3;")
                .await?;
            anyhow::bail!("reject this write")
        })
        .await
        .is_err());
    assert_eq!(value(&db).await, 2);
    db.write(|conn| async move {
        conn.execute("UPDATE values_test SET value=4", ()).await?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(value(&db).await, 4);
}

#[tokio::test]
async fn unfinished_transaction_is_rolled_back_and_rejected() {
    let db = database().await;
    assert!(db
        .write(|conn| async move {
            conn.execute_batch("BEGIN IMMEDIATE; UPDATE values_test SET value=9;")
                .await?;
            Ok(())
        })
        .await
        .is_err());
    assert_eq!(value(&db).await, 0);
}

#[tokio::test]
async fn dropped_native_transaction_preserves_the_original_error_and_rolls_back() {
    let db = database().await;
    let error = db
        .write::<_, _, ()>(|conn| async move {
            let transaction = conn.unchecked_transaction().await?;
            transaction
                .execute("UPDATE values_test SET value=7", ())
                .await?;
            anyhow::bail!("expected validation failure")
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "expected validation failure");
    db.write(|conn| async move {
        conn.execute("UPDATE values_test SET value=value+1", ())
            .await?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(value(&db).await, 1);
}
