use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct RefreshMetrics {
    since: Option<Instant>,
    polls: u64,
    full: u64,
    source_rows: u64,
    affected: u64,
    elapsed_us: u64,
    max_us: u64,
}

impl RefreshMetrics {
    pub fn observe(&mut self, source_rows: usize, affected: usize, full: bool, elapsed: Duration) {
        self.since.get_or_insert_with(Instant::now);
        self.polls += 1;
        self.full += u64::from(full);
        self.source_rows += source_rows as u64;
        self.affected += affected as u64;
        self.elapsed_us += elapsed.as_micros() as u64;
        self.max_us = self.max_us.max(elapsed.as_micros() as u64);
        if self
            .since
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(60))
        {
            tracing::info!(
                polls = self.polls,
                full = self.full,
                source_rows = self.source_rows,
                affected = self.affected,
                elapsed_us = self.elapsed_us,
                max_us = self.max_us,
                "runtime inventory work in the last minute"
            );
            *self = Self::default();
        }
    }
}
