//! Counts live drains, including nested agents, rather than saved sessions.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

#[derive(Default)]
struct Registry {
    loops: HashMap<String, usize>,
    revision: u64,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static CHANGES: OnceLock<watch::Sender<u64>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(Mutex::default)
}
fn changes() -> &'static watch::Sender<u64> {
    CHANGES.get_or_init(|| watch::channel(0).0)
}

/// A guard spans a drain (not each LLM turn). Nested calls with the same
/// session ID do not double-count; independent child session IDs do.
pub struct LoopGuard {
    id: String,
}
impl LoopGuard {
    pub fn enter(id: &str) -> Self {
        let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
        *state.loops.entry(id.into()).or_default() += 1;
        state.revision += 1;
        changes().send_replace(state.revision);
        Self { id: id.into() }
    }
}
impl Drop for LoopGuard {
    fn drop(&mut self) {
        let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = state.loops.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                state.loops.remove(&self.id);
            }
        }
        state.revision += 1;
        changes().send_replace(state.revision);
    }
}
pub fn active_ids() -> Vec<String> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .loops
        .keys()
        .cloned()
        .collect()
}
pub fn subscribe() -> watch::Receiver<u64> {
    changes().subscribe()
}

/// Publish admission and completion changes, including workloads without an LLM turn.
pub fn notify_change() {
    let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
    state.revision += 1;
    changes().send_replace(state.revision);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_drains_and_unwind_release_their_registration() {
        let id = format!("loop-test-{}", ulid::Ulid::new());
        let a = LoopGuard::enter(&id);
        let b = LoopGuard::enter(&id);
        drop(a);
        assert!(active_ids().contains(&id));
        drop(b);
        assert!(!active_ids().contains(&id));
        let _ = std::panic::catch_unwind(|| {
            let _guard = LoopGuard::enter(&id);
            panic!("test");
        });
        assert!(!active_ids().contains(&id));
    }
}
