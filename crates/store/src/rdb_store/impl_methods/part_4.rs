// Method group: platform event log, listeners, dispatch waits and the
// global scheduler gate. Adapters mirror part_1: clone arguments, run the
// domain free-function under the write lease (mutations) or read path.
use crate::rdb_store::events as ev;

macro_rules! rdb_impl_3 {
    ($($methods:tt)*) => {
        rdb_store_impl_finish! {
            $($methods)*

    async fn platform_event_append(
        &self,
        record: &opencoder_event::EventRecord,
    ) -> Result<opencoder_event::EventRecord> {
        let record = record.clone();
        self.db.write(move |db| ev::append(db, &record)).await
    }
    async fn platform_events_after(
        &self,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<opencoder_event::EventRecord>> {
        self.db.read(move |db| ev::after(db, after_seq, limit)).await
    }
    async fn platform_listener_save(
        &self,
        spec: &opencoder_event::ListenerSpec,
    ) -> Result<()> {
        let spec = spec.clone();
        self.db.write(move |db| ev::save_listener(db, &spec)).await
    }
    async fn platform_listener_delete(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.db.write(move |db| ev::delete_listener(db, &id)).await
    }
    async fn platform_listeners(&self) -> Result<Vec<opencoder_event::ListenerSpec>> {
        self.db.read(|db| ev::listeners(db)).await
    }
    async fn platform_wait_save(&self, execution_id: &str, kind_pattern: &str) -> Result<()> {
        let (execution_id, kind_pattern) = (execution_id.to_string(), kind_pattern.to_string());
        self.db
            .write(move |db| ev::save_wait(db, &execution_id, &kind_pattern))
            .await
    }
    async fn platform_wait_clear(&self, execution_id: &str) -> Result<()> {
        let execution_id = execution_id.to_string();
        self.db
            .write(move |db| ev::clear_wait(db, &execution_id))
            .await
    }
    async fn platform_due_waits(
        &self,
        kind: &opencoder_event::EventKind,
    ) -> Result<Vec<String>> {
        let kind = kind.clone();
        self.db.read(move |db| ev::due_waits(db, &kind)).await
    }
    async fn platform_consumer_cursor(&self, consumer_id: &str) -> Result<Option<i64>> {
        let consumer_id = consumer_id.to_string();
        self.db
            .read(move |db| ev::consumer_cursor(db, &consumer_id))
            .await
    }
    async fn platform_save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> Result<()> {
        let consumer_id = consumer_id.to_string();
        self.db
            .write(move |db| ev::save_consumer_cursor(db, &consumer_id, seq))
            .await
    }
    async fn scheduler_gate(&self) -> Result<Option<opencoder_event::SchedulerGate>> {
        self.db.read(|db| ev::persisted_gate(db)).await
    }
    async fn set_scheduler_gate(&self, gate: &opencoder_event::SchedulerGate) -> Result<()> {
        let gate = gate.clone();
        self.db.write(move |db| ev::save_gate(db, &gate)).await
    }
        }
    };
}
