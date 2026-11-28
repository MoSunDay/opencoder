//! Escaped connection clones retain the pool and therefore its shared
//! maintenance lease: offline maintenance is refused until the last clone
//! drops.

use opencoder_database::{
    maintenance_lock::{is_writers_active, try_acquire_exclusive, Mode},
    ConnectionLease, Database,
};
use std::path::{Path, PathBuf};

fn assert_writer_alive(path: &Path) {
    let refused = try_acquire_exclusive(path)
        .expect_err("a live writer pool must keep the shared maintenance lease");
    assert!(is_writers_active(&refused), "{refused:#}");
}

async fn path_with_pool() -> (tempfile::TempDir, PathBuf, Database) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("leased.db");
    let database = Database::open(&path).await.unwrap();
    let key = path.canonicalize().unwrap();
    (directory, key, database)
}

#[tokio::test]
async fn connect_clones_hold_the_shared_lease_until_the_last_one_drops() {
    let (_directory, key, database) = path_with_pool().await;
    let connection = database.connect().await.unwrap();
    let clone = connection.clone();
    drop(database);
    drop(connection);
    assert_writer_alive(&key);

    drop(clone);
    let maintenance = try_acquire_exclusive(&key).unwrap();
    assert_eq!(maintenance.mode(), Mode::Exclusive);
}

#[tokio::test]
async fn clones_can_escape_read_and_write_closures_without_releasing_the_lease() {
    let (_directory, key, database) = path_with_pool().await;
    let read = database
        .read(|connection| async move { Ok(connection.clone()) })
        .await
        .unwrap();
    let write = database
        .write(|connection| async move { Ok(connection.clone()) })
        .await
        .unwrap();
    drop(database);
    assert_writer_alive(&key);
    drop(read);
    assert_writer_alive(&key);
    drop(write);
    try_acquire_exclusive(&key).unwrap();
}

#[test]
fn connection_lease_is_send_and_static() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<ConnectionLease>();
}
