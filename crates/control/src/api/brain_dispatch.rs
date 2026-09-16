//! Process-local serial gates complement durable store request locks.
use tokio::sync::{Mutex, MutexGuard};
pub struct BrainGate {
    shards: [Mutex<()>; 64],
}
impl Default for BrainGate {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}
impl BrainGate {
    pub async fn lock(&self, key: &str) -> MutexGuard<'_, ()> {
        let hash = key
            .bytes()
            .fold(0usize, |n, b| n.wrapping_mul(31).wrapping_add(b as usize));
        self.shards[hash % self.shards.len()].lock().await
    }
}
