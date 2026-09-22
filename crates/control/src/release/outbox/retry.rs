//! Retry transient failures without letting old dispatches saturate the channel.
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

struct Failure {
    generation: String,
    attempts: u32,
    next: Instant,
}

#[derive(Default)]
pub(super) struct Retries {
    failures: HashMap<String, Failure>,
    seen: HashSet<String>,
}

impl Retries {
    pub fn observe(&mut self, ids: impl Iterator<Item = String>) {
        self.seen.extend(ids);
    }

    pub fn finish_scan(&mut self) {
        self.failures.retain(|id, _| self.seen.contains(id));
        self.seen.clear();
    }

    pub fn ready(&self, id: &str, generation: &str, now: Instant) -> bool {
        self.failures
            .get(id)
            .is_none_or(|failure| failure.generation != generation || now >= failure.next)
    }

    pub fn completed(&mut self, id: String, generation: String, status: u16, now: Instant) {
        if status == 202 {
            self.failures.remove(&id);
            return;
        }
        let attempts = self
            .failures
            .get(&id)
            .filter(|failure| failure.generation == generation)
            .map_or(1, |failure| failure.attempts.saturating_add(1));
        let delay = Duration::from_secs((1u64 << attempts.saturating_sub(1).min(5)).min(30));
        self.failures.insert(
            id,
            Failure {
                generation,
                attempts,
                next: now + delay,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_per_id_without_delaying_new_work_or_new_generations() {
        let mut retry = Retries::default();
        let mut now = Instant::now();
        for delay in [1, 2, 4, 8, 16, 30, 30] {
            retry.completed("old".into(), "host-1".into(), 503, now);
            let due = now + Duration::from_secs(delay);
            assert!(!retry.ready("old", "host-1", due - Duration::from_nanos(1)));
            assert!(retry.ready("old", "host-1", due));
            assert!(retry.ready("new", "host-1", now));
            assert!(retry.ready("old", "host-2", now));
            now = due;
        }
        retry.completed("old".into(), "host-2".into(), 504, now);
        assert!(retry.ready("old", "host-2", now + Duration::from_secs(1)));
        retry.completed("old".into(), "host-2".into(), 202, now);
        assert!(retry.ready("old", "host-2", now));
    }

    #[test]
    fn scan_prunes_completed_history_but_keeps_failures_seen_on_earlier_pages() {
        let mut retry = Retries::default();
        let now = Instant::now();
        for id in ["first-page", "last-page", "completed-elsewhere"] {
            retry.completed(id.into(), "host-1".into(), 503, now);
        }
        retry.observe(["first-page".into()].into_iter());
        retry.observe(["last-page".into()].into_iter());
        retry.finish_scan();
        assert!(!retry.ready("first-page", "host-1", now));
        assert!(!retry.ready("last-page", "host-1", now));
        assert!(retry.ready("completed-elsewhere", "host-1", now));
        retry.finish_scan();
        assert!(retry.failures.is_empty());
    }
}
