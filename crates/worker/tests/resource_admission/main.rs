//! A slow filesystem must not stop a current-thread node's timers/control work.
#![cfg(unix)]
#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::{
    io::Write,
    time::{Duration, Instant},
};

#[tokio::test]
async fn blocked_resource_read_does_not_starve_node_executor_or_lose_scoped_pool() {
    let (_guard, home) = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let worker = support::worker(directory.path(), support::mock()).await;
    let card = home.path().join(".opencoder/agents/blocked-card");
    std::fs::create_dir_all(&card).unwrap();
    let fifo = card.join("meta.json");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let (release, receiver) = std::sync::mpsc::channel();
    let (reading, read_started) = tokio::sync::oneshot::channel();
    let fifo_for_cleanup = fifo.clone();
    let writer = std::thread::spawn(move || {
        // Opening the writer proves the real metadata reader has arrived.
        // That read blocks until the delayed bytes arrive. The timeout also
        // makes the pre-fix case terminate so it fails rather than hanging CI.
        let mut file = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
        let _ = reading.send(Instant::now());
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        file.write_all(b"{invalid metadata}").unwrap();
    });
    let assignment = support::assignment(
        &worker,
        "agent-slow-resources",
        ExecutionKind::Agent,
        json!({"prompt":""}),
        None,
    );
    let control = async {
        let observed = tokio::time::timeout(Duration::from_secs(1), read_started).await;
        if observed.is_err() {
            // Unblock and clean the helper if the scoped pool was lost.
            let _reader = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fifo_for_cleanup)
                .unwrap();
            let _ = release.send(());
            panic!("preflight did not read its caller-scoped resource pool");
        }
        // Measure the blocked read itself, excluding durable admission setup.
        // The writer thread's timestamp still exposes an executor blocked in read().
        let started = observed.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let elapsed = started.elapsed();
        let _ = release.send(());
        elapsed
    };
    let (reply, elapsed) =
        tokio::join!(worker.handle(NodeOperation::Create { assignment }), control);
    writer.join().unwrap();
    assert!(
        elapsed < Duration::from_secs(1),
        "node executor blocked for {elapsed:?}"
    );
    assert_eq!(
        reply.status, 400,
        "caller-scoped invalid pool must be read: {reply:?}"
    );
    assert!(
        reply.body.to_string().contains("key must be a string"),
        "{reply:?}"
    );
    assert!(
        worker.indexes().await.unwrap().is_empty(),
        "failed preflight must not accept execution"
    );
    worker.shutdown().await.unwrap();
}
