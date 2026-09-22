//! Per-request phase timings; payloads and configuration never enter diagnostics.
use std::time::{Duration, Instant};

pub(in crate::operations) struct Timing {
    id: String,
    operation: &'static str,
    started: Instant,
    previous: Instant,
    phases: Vec<(&'static str, u128)>,
}

impl Timing {
    pub(in crate::operations) fn new(id: &str, operation: &'static str) -> Self {
        let now = Instant::now();
        Self {
            id: id.to_owned(),
            operation,
            started: now,
            previous: now,
            phases: Vec::new(),
        }
    }

    pub(in crate::operations) fn mark(&mut self, stage: &'static str) {
        let now = Instant::now();
        self.phases
            .push((stage, now.duration_since(self.previous).as_millis()));
        self.previous = now;
    }
}

impl Drop for Timing {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        if elapsed < Duration::from_millis(250) {
            return;
        }
        self.mark("return");
        tracing::warn!(
            execution_id = %self.id,
            operation = self.operation,
            elapsed_ms = elapsed.as_millis(),
            phases = ?self.phases,
            "slow execution admission"
        );
    }
}
