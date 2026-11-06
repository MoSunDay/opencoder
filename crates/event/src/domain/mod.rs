//! Event domain types: kinds/records, listeners and pure registry math.

pub mod consumer;
pub mod event;
pub mod gate;
pub mod listener;
pub mod registry;

pub use gate::{allows_dispatch, pause, paused, resume, resumed, SchedulerGate};
pub use event::{
    custom_kind, is_custom, new_id, new_record, rebuild_record, EventKind, EventKindError,
    EventRecord, BRAIN_LAYER_BARRIER_REACHED, BRAIN_OPERATION_TERMINAL, BRAIN_RUN_PAUSED,
    BRAIN_RUN_RESUMED, CUSTOM_KIND_PREFIX, EXECUTION_PAUSED, EXECUTION_TERMINAL,
};
pub use listener::{
    new_listener, validate, validate_id, validate_kind_pattern, ListenerError, ListenerSpec,
    TriggerAction,
};
pub use consumer::{
    validate_consumer_id, validate_cursor, ConsumerError, MAX_CONSUMER_ID_BYTES, START_CURSOR,
};
pub use registry::{listeners_for, matches, register, remove, set_enabled, RegistryError};
