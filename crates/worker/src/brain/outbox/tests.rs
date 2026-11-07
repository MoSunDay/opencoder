use super::*;
use crate::DirectoryLayout;
use opencoder_store::TursoStore;

#[tokio::test]
async fn reader_bridge_persists_legacy_ack_before_database_and_repairs_interrupted_write() {
    for legacy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let layout = DirectoryLayout::new(dir.path().join("journal"), None).unwrap();
        let mut journal = Journal::load(layout.clone()).unwrap();
        let record = serde_json::from_value(json!({"assignment": {
            "index":{"id":"agent-child","kind":"agent","node_id":"node","created_at":1,"status":"done"},
            "request":{"id":"agent-child","kind":"agent","input":{"schema_version":7,
                "payload":"unchanged", "brain_layered":{"parent":{"run_id":"brain-parent","operation_id":"op","node_id":"step","layer":1}}}}},
            "result":null,"error":null,"events":[{"kind":"done","data":{},"ts":1,"seq":1}]})).unwrap();
        journal.save(record).unwrap();
        let path = layout
            .record_path(ExecutionKind::Agent, "agent-child")
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let store = Arc::new(TursoStore::open_memory().await.unwrap());
        let conn = store.conn().await.unwrap();
        conn.execute("CREATE TRIGGER fail_ack BEFORE INSERT ON execution_report_acks BEGIN SELECT RAISE(ABORT,'injected ACK failure'); END", ()).await.unwrap();
        let ack = ReportAck {
            execution_id: "agent-child".into(),
            action: "layered_terminal".into(),
            item: String::new(),
            cursor: 1,
        };
        let store: Arc<dyn Store> = store;
        assert!(save_ack_with_format(&store, &mut journal, &ack, legacy)
            .await
            .is_err());
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved["assignment"]["request"]["input"]["payload"],
            "unchanged"
        );
        if legacy {
            assert_eq!(
                saved["annotations"]["brain_layered_ack"], 1,
                "a crash cannot leave only the new ACK representation durable during the bridge"
            );
        } else {
            assert_eq!(std::fs::read(&path).unwrap(), original);
            assert!(
                journal.has_terminal_reports(),
                "a failed durable ACK must remain pending"
            );
        }
        conn.execute("DROP TRIGGER fail_ack", ()).await.unwrap();
        let mut recovered = Journal::load(layout).unwrap();
        initialize(&store, &mut recovered).await.unwrap();
        if legacy {
            assert_eq!(store.report_acks().await.unwrap()[0].cursor, 1);
            assert!(!recovered.has_terminal_reports());
        } else {
            assert!(store.report_acks().await.unwrap().is_empty());
            assert!(recovered.has_terminal_reports());
            save_ack_with_format(&store, &mut recovered, &ack, false)
                .await
                .unwrap();
            assert!(!recovered.has_terminal_reports());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
    }
}
