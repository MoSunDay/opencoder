use std::{collections::HashMap, sync::Mutex, time::Duration};
use tokio::time::Instant;

const IDLE_GRACE: Duration = Duration::from_secs(60);

/// Keep a recently queried Runtime available between related history reads.
#[derive(Default)]
pub(super) struct RecentUse(Mutex<HashMap<String, Instant>>);

pub(super) struct RuntimeUse<'a> {
    activity: &'a RecentUse,
    runtime: String,
}

impl RecentUse {
    pub(super) fn begin(&self, runtime: &str) -> RuntimeUse<'_> {
        self.touch(runtime);
        RuntimeUse {
            activity: self,
            runtime: runtime.into(),
        }
    }

    fn touch(&self, runtime: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(runtime.into(), Instant::now());
    }

    pub(super) fn recent(&self, runtime: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(runtime)
            .is_some_and(|last| last.elapsed() < IDLE_GRACE)
    }
}

impl Drop for RuntimeUse<'_> {
    fn drop(&mut self) {
        // Cancellation and timeout also release the use guard. Start the
        // grace period after the last request finishes, not when it began.
        self.activity.touch(&self.runtime);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn grace_expires_and_later_reads_extend_only_their_runtime() {
        let activity = RecentUse::default();
        assert!(!activity.recent("history"));
        drop(activity.begin("history"));
        tokio::time::advance(Duration::from_secs(40)).await;
        drop(activity.begin("other"));
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(!activity.recent("history"));
        assert!(activity.recent("other"));
        drop(activity.begin("history"));
        assert!(activity.recent("history"));
        tokio::time::advance(IDLE_GRACE).await;
        assert!(!activity.recent("history"));
        assert!(!activity.recent("other"));
    }

    #[tokio::test(start_paused = true)]
    async fn completion_or_cancellation_starts_a_full_idle_period() {
        let activity = RecentUse::default();
        let request = activity.begin("history");
        tokio::time::advance(IDLE_GRACE * 2).await;
        drop(request);
        assert!(activity.recent("history"));
        tokio::time::advance(IDLE_GRACE - Duration::from_secs(1)).await;
        assert!(activity.recent("history"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!activity.recent("history"));
    }
}
