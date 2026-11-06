//! Durable consumer pump: cursor, gate and failure semantics.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use opencoder_event::domain::event::{new_record, EventKind, EventRecord};
use opencoder_event::domain::gate::{paused, resumed};
use opencoder_event::domain::listener::{new_listener, ListenerSpec, TriggerAction};
use opencoder_event::executor::ActionExecutor;
use opencoder_event::ledger::{EventLedger, MemoryLedger};
use opencoder_event::{run_pump, PumpRequest, PumpRun};
use serde_json::json;

/// Delegating ledger that counts cursor saves (no-op runs must not write).
#[derive(Clone)]
struct CountingLedger {
    inner: Arc<MemoryLedger>,
    cursor_saves: Arc<AtomicUsize>,
}

impl CountingLedger {
    fn new() -> Self {
        Self {
            inner: Arc::new(MemoryLedger::new()),
            cursor_saves: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn append_seq(&self, seq: i64) {
        let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        let record = new_record(
            EventKind::new("event.custom.tick"),
            json!({"seq": seq}),
            "test",
            seq,
            now,
        );
        self.inner.append(&record).await.unwrap();
    }
}

#[async_trait::async_trait]
impl EventLedger for CountingLedger {
    async fn append(&self, record: &EventRecord) -> anyhow::Result<()> {
        self.inner.append(record).await
    }
    async fn events_after(&self, after_seq: i64, limit: u32) -> anyhow::Result<Vec<EventRecord>> {
        self.inner.events_after(after_seq, limit).await
    }
    async fn save_listener(&self, spec: &ListenerSpec) -> anyhow::Result<()> {
        self.inner.save_listener(spec).await
    }
    async fn delete_listener(&self, id: &str) -> anyhow::Result<()> {
        self.inner.delete_listener(id).await
    }
    async fn list_listeners(&self) -> anyhow::Result<Vec<ListenerSpec>> {
        self.inner.list_listeners().await
    }
    async fn save_wait(&self, execution_id: &str, kind_pattern: &str) -> anyhow::Result<()> {
        self.inner.save_wait(execution_id, kind_pattern).await
    }
    async fn clear_wait(&self, execution_id: &str) -> anyhow::Result<()> {
        self.inner.clear_wait(execution_id).await
    }
    async fn due_waits(&self, kind: &EventKind) -> anyhow::Result<Vec<String>> {
        self.inner.due_waits(kind).await
    }
    async fn consumer_cursor(&self, consumer_id: &str) -> anyhow::Result<Option<i64>> {
        self.inner.consumer_cursor(consumer_id).await
    }
    async fn save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> anyhow::Result<()> {
        self.cursor_saves.fetch_add(1, Ordering::SeqCst);
        self.inner.save_consumer_cursor(consumer_id, seq).await
    }
}

/// Records dispatched event seqs; fails the action of `fail_message`
/// (listener's notify message) while `failing` is set.
struct RecordingExecutor {
    dispatched: Mutex<Vec<i64>>,
    fail_message: Option<&'static str>,
    failing: Arc<AtomicBool>,
}

impl RecordingExecutor {
    fn new() -> Self {
        Self {
            dispatched: Mutex::new(Vec::new()),
            fail_message: None,
            failing: Arc::new(AtomicBool::new(false)),
        }
    }

    fn flaky(fail_message: &'static str, failing: Arc<AtomicBool>) -> Self {
        Self {
            dispatched: Mutex::new(Vec::new()),
            fail_message: Some(fail_message),
            failing,
        }
    }

    fn dispatched(&self) -> Vec<i64> {
        self.dispatched.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ActionExecutor for RecordingExecutor {
    async fn execute(&self, action: &TriggerAction, event: &EventRecord) -> anyhow::Result<()> {
        self.dispatched.lock().unwrap().push(event.seq);
        let targeted = matches!(action, TriggerAction::Notify { message }
            if Some(message.as_str()) == self.fail_message);
        if targeted && self.failing.load(Ordering::SeqCst) {
            anyhow::bail!("listener unavailable");
        }
        Ok(())
    }
}

fn notify(id: &str) -> ListenerSpec {
    new_listener(
        id,
        "event.custom.*",
        None,
        TriggerAction::Notify {
            message: format!("m-{id}"),
        },
    )
}

fn request<'a>(
    consumer_id: &'a str,
    gate: &'a opencoder_event::SchedulerGate,
    limit: u32,
) -> PumpRequest<'a> {
    PumpRequest {
        consumer_id,
        gate,
        batch_limit: limit,
    }
}

async fn pump_open(
    ledger: &CountingLedger,
    executor: &dyn ActionExecutor,
    registry: &[ListenerSpec],
    consumer_id: &str,
    limit: u32,
) -> PumpRun {
    let open = resumed(1);
    run_pump(
        ledger,
        executor,
        registry,
        request(consumer_id, &open, limit),
    )
    .await
    .unwrap()
}

async fn ledger_with(events: i64) -> CountingLedger {
    let ledger = CountingLedger::new();
    for seq in 1..=events {
        ledger.append_seq(seq).await;
    }
    ledger
}

#[tokio::test]
async fn paused_gate_returns_paused_without_dispatching_or_advancing() {
    let ledger = ledger_with(3).await;
    let executor = RecordingExecutor::new();
    let gate = paused("maintenance", 5);
    let run = run_pump(
        &ledger,
        &executor,
        &[notify("l1")],
        request("pump", &gate, 10),
    )
    .await
    .unwrap();
    assert!(matches!(run, PumpRun::Paused));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 0);
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
    assert!(executor.dispatched().is_empty());
}

#[tokio::test]
async fn successful_batch_dispatches_all_and_saves_cursor_once() {
    let ledger = ledger_with(3).await;
    let executor = RecordingExecutor::new();
    let run = pump_open(&ledger, &executor, &[notify("l1")], "pump", 10).await;
    assert!(matches!(
        run,
        PumpRun::Advanced {
            to_seq: 3,
            events: 3,
            actions: 3
        }
    ));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(3));
    assert_eq!(executor.dispatched(), vec![1, 2, 3]);
}

#[tokio::test]
async fn failing_listener_keeps_cursor_then_retry_advances() {
    let ledger = ledger_with(2).await;
    let failing = Arc::new(AtomicBool::new(true));
    let executor = RecordingExecutor::flaky("m-l2", Arc::clone(&failing));
    let registry = [notify("l1"), notify("l2")];

    // l2 fails on the first event: report seq + listener id, keep cursor.
    let run = pump_open(&ledger, &executor, &registry, "pump", 10).await;
    match run {
        PumpRun::Failed { failures } => {
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].event_seq, 1);
            assert_eq!(failures[0].listener_id, "l2");
            assert!(failures[0].error.contains("listener unavailable"));
        }
        other => panic!("expected failed, got {other:?}"),
    }
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 0);
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
    // The pump stops at the first failing event: seq 2 was not dispatched.
    assert_eq!(executor.dispatched(), vec![1, 1]);

    // Retry after recovery: the batch replays fully and advances.
    failing.store(false, Ordering::SeqCst);
    let run = pump_open(&ledger, &executor, &registry, "pump", 10).await;
    assert!(matches!(
        run,
        PumpRun::Advanced {
            to_seq: 2,
            events: 2,
            actions: 4
        }
    ));
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(2));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn empty_log_is_idle_and_writes_nothing() {
    let ledger = ledger_with(0).await;
    let executor = RecordingExecutor::new();
    let run = pump_open(&ledger, &executor, &[notify("l1")], "pump", 10).await;
    assert!(matches!(run, PumpRun::Idle));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 0);
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
    assert!(executor.dispatched().is_empty());
}

#[tokio::test]
async fn batch_limit_chunks_progress_then_goes_idle() {
    let ledger = ledger_with(5).await;
    let executor = RecordingExecutor::new();

    let first = pump_open(&ledger, &executor, &[notify("l1")], "pump", 2).await;
    assert!(matches!(
        first,
        PumpRun::Advanced {
            to_seq: 2,
            events: 2,
            actions: 2
        }
    ));
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(2));

    let second = pump_open(&ledger, &executor, &[notify("l1")], "pump", 2).await;
    assert!(matches!(
        second,
        PumpRun::Advanced {
            to_seq: 4,
            events: 2,
            actions: 2
        }
    ));
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(4));

    let third = pump_open(&ledger, &executor, &[notify("l1")], "pump", 2).await;
    assert!(matches!(
        third,
        PumpRun::Advanced {
            to_seq: 5,
            events: 1,
            actions: 1
        }
    ));
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(5));

    let drained = pump_open(&ledger, &executor, &[notify("l1")], "pump", 2).await;
    assert!(matches!(drained, PumpRun::Idle));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 3);
    assert_eq!(executor.dispatched(), vec![1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn paused_after_progress_keeps_saved_cursor_and_resumes_from_it() {
    let ledger = ledger_with(3).await;
    let executor = RecordingExecutor::new();
    let run = pump_open(&ledger, &executor, &[notify("l1")], "pump", 2).await;
    assert!(matches!(run, PumpRun::Advanced { to_seq: 2, .. }));

    let gate = paused("maintenance", 9);
    let run = run_pump(
        &ledger,
        &executor,
        &[notify("l1")],
        request("pump", &gate, 10),
    )
    .await
    .unwrap();
    assert!(matches!(run, PumpRun::Paused));
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), Some(2));
    assert_eq!(executor.dispatched(), vec![1, 2]);

    // Resume: the remaining event replays from the held cursor.
    let run = pump_open(&ledger, &executor, &[notify("l1")], "pump", 10).await;
    assert!(matches!(
        run,
        PumpRun::Advanced {
            to_seq: 3,
            events: 1,
            actions: 1
        }
    ));
    assert_eq!(executor.dispatched(), vec![1, 2, 3]);
}

#[tokio::test]
async fn invalid_consumer_id_is_rejected_before_any_work() {
    let ledger = ledger_with(1).await;
    let executor = RecordingExecutor::new();
    let gate = resumed(1);
    let too_long = "a".repeat(200);
    for bad in ["", "id with space", "a/b", too_long.as_str()] {
        let result = run_pump(&ledger, &executor, &[notify("l1")], request(bad, &gate, 10)).await;
        assert!(result.is_err(), "consumer id {bad:?} must be rejected");
    }
    assert_eq!(ledger.cursor_saves.load(Ordering::SeqCst), 0);
    assert!(executor.dispatched().is_empty());
}
