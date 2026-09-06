//! Scoped tools supplied by an embedding application for one running session.
use opencoder_core::ToolArc;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
struct Entry {
    generation: u64,
    tools: Vec<ToolArc>,
}
#[derive(Default)]
struct Registry {
    generation: u64,
    entries: HashMap<String, Entry>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(Mutex::default)
}
pub struct Registration {
    id: String,
    generation: u64,
}
pub fn register(id: &str, tools: Vec<ToolArc>) -> Registration {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry.generation += 1;
    let generation = registry.generation;
    registry
        .entries
        .insert(id.into(), Entry { generation, tools });
    Registration {
        id: id.into(),
        generation,
    }
}
pub(crate) fn tools(id: &str) -> Vec<ToolArc> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .get(id)
        .map(|e| e.tools.clone())
        .unwrap_or_default()
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        if registry
            .entries
            .get(&self.id)
            .is_some_and(|e| e.generation == self.generation)
        {
            registry.entries.remove(&self.id);
        }
    }
}
