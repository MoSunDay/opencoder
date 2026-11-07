// Method group 4: platform event log, pause gate and dispatch waits.
//
// Every method has a default that refuses the call, so backends without
// platform-event support keep compiling; the rdb backend overrides all.
macro_rules! store_contract_3 {
    ($($methods:tt)*) => {
        store_contract_finish! {
            $($methods)*
    /// Append one platform event; returns the stored record with its seq.
    async fn platform_event_append(
        &self,
        _record: &opencoder_event::EventRecord,
    ) -> Result<opencoder_event::EventRecord> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    /// Events with `seq > after_seq`, ascending, at most `limit`.
    async fn platform_events_after(
        &self,
        _after_seq: i64,
        _limit: u32,
    ) -> Result<Vec<opencoder_event::EventRecord>> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_listener_save(
        &self,
        _spec: &opencoder_event::ListenerSpec,
    ) -> Result<()> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_listener_delete(&self, _id: &str) -> Result<()> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_listeners(&self) -> Result<Vec<opencoder_event::ListenerSpec>> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_wait_save(&self, _execution_id: &str, _kind_pattern: &str) -> Result<()> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_wait_clear(&self, _execution_id: &str) -> Result<()> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_due_waits(
        &self,
        _kind: &opencoder_event::EventKind,
    ) -> Result<Vec<String>> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    /// Durable replay cursor of `consumer_id` (`None` = never saved).
    /// Unsupported backends refuse instead of reporting a fake start
    /// cursor, which would silently replay the whole log.
    async fn platform_consumer_cursor(&self, _consumer_id: &str) -> Result<Option<i64>> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    async fn platform_save_consumer_cursor(&self, _consumer_id: &str, _seq: i64) -> Result<()> {
        anyhow::bail!("platform events are unsupported by {}", self.backend_name())
    }
    /// Current scheduler gate; `None` when the backend has no gate.
    async fn scheduler_gate(&self) -> Result<Option<opencoder_event::SchedulerGate>> {
        Ok(None)
    }
    async fn set_scheduler_gate(&self, _gate: &opencoder_event::SchedulerGate) -> Result<()> {
        anyhow::bail!("scheduler gate is unsupported by {}", self.backend_name())
    }
        }
    };
}
