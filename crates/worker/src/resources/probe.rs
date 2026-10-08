//! One detached filesystem probe; snapshots never wait for its I/O.
use crate::Worker;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub type MountHealthReader = Arc<dyn Fn(Option<&Path>) -> anyhow::Result<()> + Send + Sync>;
const INTERVAL: Duration = Duration::from_secs(2);
const DEADLINE: Duration = Duration::from_secs(5);
const FRESHNESS: Duration = Duration::from_secs(10);

struct State {
    path: Option<PathBuf>,
    started: Option<Instant>,
    updated: Instant,
    error: Option<String>,
}

impl State {
    fn error(&self, now: Instant) -> Option<String> {
        if self
            .started
            .is_some_and(|at| now.duration_since(at) >= DEADLINE)
        {
            return Some("agent resource probe timed out".into());
        }
        if now.duration_since(self.updated) >= FRESHNESS {
            return Some("agent resource health is stale".into());
        }
        self.error.clone()
    }

    fn observe_path(&mut self, path: Option<PathBuf>) {
        if self.path != path {
            self.path = path;
            self.error = Some("agent resource mount has not been checked".into());
        }
    }

    fn finish(&mut self, now: Instant, result: anyhow::Result<()>) {
        let elapsed = self.started.take().map(|at| now.duration_since(at));
        self.updated = now;
        self.error = if elapsed.is_some_and(|elapsed| elapsed >= DEADLINE) {
            Some("agent resource probe timed out".into())
        } else {
            result.err().map(|error| format!("{error:#}"))
        };
    }
}

pub(crate) struct MountProbe(Arc<Mutex<State>>);

impl MountProbe {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self(Arc::new(Mutex::new(State {
            error: path
                .as_ref()
                .map(|_| "agent resource mount has not been checked".into()),
            path,
            started: None,
            updated: Instant::now(),
        })))
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.0.lock().unwrap().error(Instant::now())
    }

    pub(crate) fn start(&self, worker: &Worker) -> anyhow::Result<()> {
        let state = self.0.clone();
        let weak = Arc::downgrade(&worker.inner);
        let stop = worker.inner.stopping.clone();
        let config_home = opencoder_core::config::ScopedConfigHome::current();
        std::thread::Builder::new()
            .name("resource-health".into())
            .spawn(move || {
                let _isolation = config_home.map(opencoder_core::config::scoped_config_home);
                while !stop.is_cancelled() {
                    let Some(inner) = weak.upgrade() else { return };
                    let current = Worker { inner };
                    let workdir = crate::brain::workdir::node_workdir(&current);
                    let reader = current.inner.runtime.mount_health.clone();
                    // A blocked filesystem call must not retain the Worker or delay shutdown.
                    drop(current);
                    state.lock().unwrap().started = Some(Instant::now());
                    let result = opencoder_core::agent::scope::with_root_sync(None, || {
                        opencoder_core::Config::load(&workdir).map_err(anyhow::Error::from)
                    })
                    .and_then(|config| {
                        let path = config.agent.agents_dir;
                        state.lock().unwrap().observe_path(path.clone());
                        reader(path.as_deref())
                    });
                    state.lock().unwrap().finish(Instant::now(), result);
                    opencoder_session::loop_registry::notify_change();
                    for _ in 0..20 {
                        if stop.is_cancelled() {
                            return;
                        }
                        std::thread::sleep(INTERVAL / 20);
                    }
                }
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_staleness_and_path_changes_fail_closed() {
        let now = Instant::now();
        let mut state = State {
            path: None,
            started: None,
            updated: now,
            error: None,
        };
        assert_eq!(state.error(now), None);
        state.observe_path(Some("/mount/new".into()));
        assert!(state.error(now).unwrap().contains("not been checked"));
        state.started = Some(now);
        assert!(state.error(now + DEADLINE).unwrap().contains("timed out"));
        state.finish(now + DEADLINE, Ok(()));
        assert!(state.error(now + DEADLINE).unwrap().contains("timed out"));
        state.started = Some(now + DEADLINE);
        state.finish(now + DEADLINE + Duration::from_millis(1), Ok(()));
        assert_eq!(state.error(now + DEADLINE + Duration::from_millis(1)), None);
        assert!(state
            .error(now + DEADLINE + FRESHNESS + Duration::from_millis(1))
            .unwrap()
            .contains("stale"));
    }
}
