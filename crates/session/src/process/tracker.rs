//! Process-wide completion tracking for hidden workload owners.

use super::{pidfd::start_time, SignalTarget};
use anyhow::Result;
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Identity {
    pid: u32,
    start_time: u64,
}

struct Tracker {
    active: Mutex<HashSet<Identity>>,
    revision: watch::Sender<u64>,
}

fn tracker() -> &'static Tracker {
    static TRACKER: OnceLock<Tracker> = OnceLock::new();
    TRACKER.get_or_init(|| {
        let (revision, _) = watch::channel(0);
        Tracker {
            active: Mutex::new(HashSet::new()),
            revision,
        }
    })
}

pub(super) fn register(target: &SignalTarget) {
    let identity = Identity {
        pid: target.pid(),
        start_time: target.start_time(),
    };
    let state = tracker();
    state.active.lock().unwrap().insert(identity);
    publish(state);
    tokio::spawn(async move {
        while start_time(identity.pid) == Some(identity.start_time) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let state = tracker();
        if state.active.lock().unwrap().remove(&identity) {
            publish(state);
        }
    });
}

fn publish(state: &Tracker) {
    state
        .revision
        .send_modify(|revision| *revision = revision.wrapping_add(1));
}

/// Number of exact PID/start-time owner identities that have not exited.
pub fn active_owned_processes() -> usize {
    tracker().active.lock().unwrap().len()
}

/// Wait until every hidden owner has finished descendant/container cleanup.
/// A watch generation supports any number of simultaneous drain waiters
/// without notification loss.
pub async fn wait_for_owned_processes(deadline: tokio::time::Instant) -> Result<()> {
    let state = tracker();
    let mut changes = state.revision.subscribe();
    loop {
        if state.active.lock().unwrap().is_empty() {
            return Ok(());
        }
        tokio::time::timeout_at(deadline, changes.changed())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "node shutdown timed out waiting for {} owned process supervisors",
                    active_owned_processes()
                )
            })?
            .map_err(|_| anyhow::anyhow!("owned process tracker closed"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    #[tokio::test]
    async fn timeout_is_explicit_then_multiple_waiters_observe_exact_owner_exit() {
        let mut child = Command::new("sh")
            .args(["-c", "while :; do sleep 1; done"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let target = SignalTarget::open(child.id().unwrap()).unwrap();
        register(&target);
        let error =
            wait_for_owned_processes(tokio::time::Instant::now() + Duration::from_millis(20))
                .await
                .unwrap_err()
                .to_string();
        assert!(error.contains("1 owned process supervisors"), "{error}");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let left = wait_for_owned_processes(deadline);
        let right = wait_for_owned_processes(deadline);
        child.start_kill().unwrap();
        child.wait().await.unwrap();
        let (left, right) = tokio::join!(left, right);
        left.unwrap();
        right.unwrap();
        assert_eq!(active_owned_processes(), 0);
    }
}
