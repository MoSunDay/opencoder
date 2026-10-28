use super::{assignment, native_fixture, open};
use crate::{journal::Record, lifecycle::StopIntent};
use futures::poll;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn durable_cancel_signals_before_a_queued_journal_reader_can_block_it() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = open(root.path()).await;
    let id = "agent-stop-signal";
    let mut owned = assignment(&worker, id, ExecutionKind::Agent);
    owned.index.status = ExecutionStatus::Running;
    let record = Record {
        assignment: owned,
        annotations: serde_json::Value::Null,
        result: serde_json::Value::Null,
        error: None,
        events: vec![],
        lifecycle: Default::default(),
        queue: None,
    };
    worker.inner.journal.lock().await.save(record).unwrap();
    let token = CancellationToken::new();
    worker
        .inner
        .active
        .lock()
        .await
        .insert(id.into(), token.clone());

    let first = worker.inner.journal.lock().await;
    let mut stopping = Box::pin(crate::operations::durable_stop(
        &worker,
        id,
        StopIntent::Cancel,
    ));
    assert!(poll!(&mut stopping).is_pending());
    let mut other_reader = Box::pin(worker.inner.journal.lock());
    assert!(poll!(&mut other_reader).is_pending());
    drop(first);

    // The record may now require asynchronous disk IO while holding journal.
    // That is allowed; letting the queued reader acquire journal before the
    // durable stop signals cancellation is not.
    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            biased;
            reply = &mut stopping => reply.unwrap(),
            _ = &mut other_reader => panic!("durable cancellation requeued behind another journal user"),
        }
    }).await.unwrap();
    assert_eq!(reply.status, 200);
    assert!(token.is_cancelled());
    let journal = other_reader.await;
    assert_eq!(
        journal
            .get(id)
            .await
            .unwrap()
            .expect("execution exists")
            .lifecycle
            .stop_intent,
        Some(StopIntent::Cancel)
    );
    assert_eq!(
        journal
            .get(id)
            .await
            .unwrap()
            .expect("execution exists")
            .assignment
            .index
            .status,
        ExecutionStatus::Cancelling
    );
    drop(journal);
    worker.inner.active.lock().await.remove(id);
    crate::state::release_execution_state(&worker.inner, id).await;
}

#[tokio::test]
async fn native_cancel_reaps_runc_and_descendants_without_model_calls() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let native = native_fixture::ContainerFixture::open(&root.path().join("node"));
    let mut config = opencoder_core::Config::default();
    native.configure(&mut config);
    std::fs::create_dir_all(root.path().join("work")).unwrap();
    std::fs::write(
        root.path().join("work/opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let source = root.path().join("hold.c");
    let binary = root.path().join("hold");
    std::fs::write(&source, "#include <stdio.h>\n#include <unistd.h>\nint main(void) { if (fork()==0) { for (;;) pause(); } FILE *f=fopen(\"started\",\"w\"); if (!f) return 1; fputs(\"ready\",f); fclose(f); for (;;) pause(); }").unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-static", "-s"])
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .status()
        .unwrap()
        .success());
    opencoder_dag_binary::save_binary_version(
        &native.pool,
        "hold",
        "fixture",
        &std::fs::read(binary).unwrap(),
    )
    .unwrap();
    let worker = open(root.path()).await;
    let id = "dag-stop-native";
    let mut request = assignment(&worker, id, ExecutionKind::Dag);
    request.definition = Some(
        serde_json::json!({"name":"native-stop","steps":[{"name":"hold","timeout_secs":60,"kind":{"type":"binary","resource":"hold"}}]}),
    );
    let accepted = crate::operations::create::create(&worker, request)
        .await
        .unwrap();
    assert_eq!(accepted.status, 200);
    let record = worker
        .inner
        .journal
        .lock()
        .await
        .get(id)
        .await
        .unwrap()
        .expect("execution exists")
        .clone();
    let run = crate::layout::dag::accepted_parent(&record)
        .unwrap()
        .join(id);
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while !run.join("upper/hold/started").is_file() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real runc child did not start");
    let reply = crate::operations::durable_stop(&worker, id, StopIntent::Cancel)
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["status"], "cancelling");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while worker
            .inner
            .journal
            .lock()
            .await
            .get(id)
            .await
            .unwrap()
            .expect("execution exists")
            .assignment
            .index
            .status
            != ExecutionStatus::Cancelled
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled runc execution did not converge");
    assert!(std::fs::read_dir(run.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
    worker.shutdown().await.unwrap();
}
