use super::*;

fn record(id: &str, status: &str, bytes: usize) -> Record {
    serde_json::from_value(json!({"assignment":{
        "index":{"id":id,"kind":"agent","created_at":1,"node_id":"node-test","status":status},
        "request":{"id":id,"kind":"agent","input":{"payload":"x".repeat(bytes)}}},
        "annotations":{},"result":{"answer":"preserved"},"error":null,"events":[]
    }))
    .unwrap()
}

fn journal(dir: &tempfile::TempDir) -> Journal {
    Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap()
}

#[tokio::test]
async fn history_stays_on_disk_and_cold_reads_keep_complete_records() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    for index in 0..32 {
        journal
            .save(record(&format!("agent-history-{index}"), "done", 64 * 1024))
            .unwrap();
    }
    drop(journal);
    let journal = Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    assert_eq!(journal.summaries.len(), 32);
    assert!(
        journal.records.is_empty(),
        "unrelated history must not enter the report projection"
    );
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    let restored = journal.get("agent-history-31").await.unwrap().unwrap();
    assert_eq!(
        restored.assignment.request.input["payload"]
            .as_str()
            .unwrap()
            .len(),
        64 * 1024
    );
    assert_eq!(restored.result["answer"], "preserved");
    assert_eq!(journal.cache.lock().unwrap().usage().1, 1);
}

#[tokio::test]
async fn cache_is_bounded_by_bytes_and_count_and_large_records_bypass_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    *journal.cache.lock().unwrap() = cache::Cache::new(4096, 2);
    for id in ["agent-a", "agent-b", "agent-c", "agent-large"] {
        journal
            .save(record(
                id,
                "done",
                if id == "agent-large" { 8192 } else { 1024 },
            ))
            .unwrap();
    }
    for id in ["agent-a", "agent-b", "agent-a", "agent-c"] {
        assert!(journal.get(id).await.unwrap().is_some());
        let (bytes, count) = journal.cache.lock().unwrap().usage();
        assert!(bytes <= 4096 && count <= 2);
    }
    assert_eq!(journal.cache.lock().unwrap().usage().1, 2);
    let bytes = journal.cache.lock().unwrap().usage().0;
    assert_eq!(
        journal
            .get("agent-large")
            .await
            .unwrap()
            .unwrap()
            .assignment
            .request
            .input["payload"]
            .as_str()
            .unwrap()
            .len(),
        8192
    );
    assert_eq!(journal.cache.lock().unwrap().usage(), (bytes, 2));
}

#[tokio::test]
async fn save_and_forget_invalidate_cold_cache_and_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    journal.save(record("agent-a", "idle", 1024)).unwrap();
    let old = journal.ticket("agent-a").unwrap().unwrap();
    assert!(journal.get("agent-a").await.unwrap().is_some());
    let mut next = journal.get("agent-a").await.unwrap().unwrap();
    next.result = json!({"answer":"new"});
    journal.save(next).unwrap();
    assert_ne!(old.revision, journal.summaries["agent-a"].revision);
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    assert_eq!(
        journal.get("agent-a").await.unwrap().unwrap().result["answer"],
        "new"
    );
    assert!(journal.forget("agent-a").unwrap());
    assert!(journal.get("agent-a").await.unwrap().is_none());
    assert!(old.read_sync().is_err());
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
}

#[tokio::test]
async fn warm_cache_does_not_hide_missing_corrupt_or_replaced_records() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    journal.save(record("agent-a", "done", 1024)).unwrap();
    journal.get("agent-a").await.unwrap().unwrap();
    let path = journal
        .layout
        .record_path(ExecutionKind::Agent, "agent-a")
        .unwrap();
    std::fs::write(&path, b"broken").unwrap();
    assert!(journal.get("agent-a").await.is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(
        journal.get("agent-a").await.is_err(),
        "indexed missing data is not an absent execution"
    );
    assert!(journal.get("agent-unknown").await.unwrap().is_none());
    #[cfg(unix)]
    {
        let other = dir.path().join("other.json");
        std::fs::write(
            &other,
            serde_json::to_vec(&record("agent-a", "done", 1024)).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(other, &path).unwrap();
        assert!(journal.get("agent-a").await.is_err());
    }
}

#[tokio::test]
async fn failed_save_preserves_summary_cache_and_report_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    journal.save(record("agent-a", "idle", 1024)).unwrap();
    let original = journal.get("agent-a").await.unwrap().unwrap();
    let revision = journal.summaries["agent-a"].revision;
    let path = journal
        .layout
        .record_path(ExecutionKind::Agent, "agent-a")
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let mut changed = original.clone();
    changed.assignment.index.status = ExecutionStatus::Done;
    assert!(journal.save(changed).is_err());
    assert_eq!(journal.summaries["agent-a"].revision, revision);
    assert_eq!(
        journal.summaries["agent-a"].index.status,
        ExecutionStatus::Idle
    );
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(
        journal
            .get("agent-a")
            .await
            .unwrap()
            .unwrap()
            .assignment
            .index
            .status,
        ExecutionStatus::Idle
    );
}

#[tokio::test]
async fn report_ack_preserves_full_payload_in_both_persistence_modes() {
    for persist in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = journal(&dir);
        let mut leaf = record("agent-leaf", "done", 8192);
        leaf.assignment.request.input["schema_version"] = json!(7);
        leaf.assignment.request.input["brain_layered"] = json!({"parent":{
            "run_id":"brain-parent","operation_id":"operation-a","node_id":"step-a","layer":1
        }});
        leaf.assignment.definition = Some(json!({"frozen":"y".repeat(8192)}));
        leaf.events.push(SseEvt {
            seq: Some(3),
            ts: 1,
            kind: "done".into(),
            data: json!({"retained":true}),
        });
        journal.save(leaf.clone()).unwrap();
        journal.get("agent-leaf").await.unwrap().unwrap();
        let revision = journal.summaries["agent-leaf"].revision;
        journal
            .apply_report_ack(
                &opencoder_store::reporting::ReportAck {
                    execution_id: "agent-leaf".into(),
                    action: "layered_terminal".into(),
                    item: String::new(),
                    cursor: 3,
                },
                persist,
            )
            .unwrap();
        assert_ne!(revision, journal.summaries["agent-leaf"].revision);
        let loaded = journal.get("agent-leaf").await.unwrap().unwrap();
        assert_eq!(
            loaded.assignment.request.input,
            leaf.assignment.request.input
        );
        assert_eq!(loaded.assignment.definition, leaf.assignment.definition);
        assert_eq!(loaded.result, leaf.result);
        assert_eq!(loaded.events[0].data, leaf.events[0].data);
        assert_eq!(loaded.annotations["brain_layered_ack"], 3);
        assert!(journal.terminal_frames(None, 64).unwrap().is_empty());
        journal.save(loaded).unwrap();
        drop(journal);
        let reloaded =
            Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
        let loaded = reloaded.get("agent-leaf").await.unwrap().unwrap();
        assert_eq!(
            loaded.assignment.request.input,
            leaf.assignment.request.input
        );
        assert_eq!(loaded.assignment.definition, leaf.assignment.definition);
        assert_eq!(loaded.annotations["brain_layered_ack"], 3);
        assert!(reloaded.terminal_frames(None, 64).unwrap().is_empty());
    }
}

#[tokio::test]
async fn recovery_changes_only_lifecycle_and_keeps_frozen_payload() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    for id in ["agent-running", "agent-cancel"] {
        let mut running = record(id, "running", 8192);
        if id == "agent-cancel" {
            running.lifecycle.stop_intent = Some(StopIntent::Cancel);
        }
        journal.save(running).unwrap();
    }
    drop(journal);
    let recovered = Journal::open(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    for (id, expected) in [
        ("agent-running", ExecutionStatus::Interrupted),
        ("agent-cancel", ExecutionStatus::Cancelled),
    ] {
        let loaded = recovered.get(id).await.unwrap().unwrap();
        assert_eq!(loaded.assignment.index.status, expected);
        assert_eq!(
            loaded.assignment.request.input["payload"]
                .as_str()
                .unwrap()
                .len(),
            8192
        );
        assert_eq!(loaded.result["answer"], "preserved");
        assert_eq!(
            loaded.events.last().unwrap().data["status"],
            expected.as_str()
        );
    }
}

#[tokio::test]
async fn concurrent_read_retries_save_and_observes_forget_without_holding_journal_lock() {
    let home = tempfile::tempdir().unwrap();
    let _guard = opencoder_core::config::scoped_config_home(home.path().into());
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let worker = crate::Worker::open(
        crate::WorkerOptions {
            name: "memory-test".into(),
            workdir,
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(std::sync::Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let slots = {
        let mut journal = worker.inner.journal.lock().await;
        journal.save(record("agent-a", "idle", 1024)).unwrap();
        journal.read_slots.clone()
    };
    for (inspection, delete) in [(false, false), (false, true), (true, false), (true, true)] {
        if !delete {
            worker
                .inner
                .journal
                .lock()
                .await
                .save(record("agent-a", "idle", 1024))
                .unwrap();
        }
        let permits = slots.clone().acquire_many_owned(4).await.unwrap();
        let read = async {
            if inspection {
                worker
                    .read_inspection("agent-a")
                    .await
                    .map(|record| record.map(|r| (r.index.status, r.result)))
            } else {
                worker
                    .read_record("agent-a")
                    .await
                    .map(|record| record.map(|r| (r.assignment.index.status, r.result.clone())))
            }
        };
        tokio::pin!(read);
        assert!(futures::poll!(&mut read).is_pending());
        // A blocked disk read must not retain the mutex needed by a writer.
        let mut journal = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            worker.inner.journal.lock(),
        )
        .await
        .unwrap();
        if delete {
            journal.forget("agent-a").unwrap();
        } else {
            let mut updated = record("agent-a", "done", 2048);
            updated.result = json!({"answer":"updated"});
            journal.save(updated).unwrap();
        }
        drop(journal);
        drop(permits);
        let result = read.await.unwrap();
        if delete {
            assert!(result.is_none());
        } else {
            let result = result.unwrap();
            assert_eq!(result.0, ExecutionStatus::Done);
            assert_eq!(result.1["answer"], "updated");
        }
    }
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn historical_brain_intents_are_not_resident_report_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = journal(&dir);
    for n in 0..20 {
        let mut root = record(&format!("brain-history-{n}"), "done", 0);
        root.assignment.index.kind = ExecutionKind::Brain;
        root.assignment.request.kind = ExecutionKind::Brain;
        root.assignment.request.input = json!({"schema_version":7,"layered_request":{}});
        root.annotations = json!({"layered_intent":{"frozen":"x".repeat(64 * 1024)}});
        journal.save(root).unwrap();
    }
    drop(journal);
    let journal = Journal::load(DirectoryLayout::new(dir.path().into(), None).unwrap()).unwrap();
    assert_eq!(journal.records.len(), 20);
    assert!(journal
        .records
        .values()
        .all(|r| r.annotations.get("layered_intent").is_none()));
    assert_eq!(journal.cache.lock().unwrap().usage(), (0, 0));
    let original = journal.get("brain-history-19").await.unwrap().unwrap();
    assert_eq!(
        original.annotations["layered_intent"]["frozen"]
            .as_str()
            .unwrap()
            .len(),
        64 * 1024
    );
}
