mod admission;
mod health;
mod scheduling;
pub(crate) use scheduling::SchedulingState;

pub(crate) use admission::AdmissionState;
pub(crate) use health::capacity_error;
pub use health::{HealthReader, StorageCapacity};

use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct DrainPolicy {
    pub natural_grace: Duration,
    pub cleanup_grace: Duration,
}

impl Default for DrainPolicy {
    fn default() -> Self {
        Self {
            natural_grace: Duration::from_secs(10 * 60),
            cleanup_grace: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
pub struct WorkerRuntime {
    pub drain: DrainPolicy,
    pub health: HealthReader,
}

impl Default for WorkerRuntime {
    fn default() -> Self {
        Self {
            drain: DrainPolicy::default(),
            health: Arc::new(health::read_storage_capacity),
        }
    }
}
