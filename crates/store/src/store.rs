pub mod dag_snapshot;
mod message_projection;

use anyhow::Result;
use async_trait::async_trait;

use opencoder_core::Message;

use crate::types::{
    ImportReport, MessageChunkPage, MessageRow, SessionEventPage, SessionEventRecord,
    SessionFilter, SessionInput, SessionListItem, SessionMeta, SessionPatch, SubagentTaskRecord,
};
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainVectorHit,
    BrainVectorWrite, ScheduleRunRecord, TeamTopicRunRecord, TodoEventPage, TodoEventRecord,
    TodoItemRecord, TodoWorkflowDetail, TodoWorkflowRecord, TodoWorkflowSummary,
};

/// Storage abstraction — the single seam that isolates upper layers from the
/// concrete engine (currently RocksDB via `rdb_store`).
///
/// Upper-layer code depends on `Arc<dyn Store>`; the concrete impl lives in
/// `rdb_store`.
// Rust traits and trait implementations cannot be split into partial items.
// Assemble method groups first so async_trait sees one complete declaration.
macro_rules! store_contract_finish {
    ($($methods:tt)*) => {
        #[async_trait]
        // async_trait adds must_use to Future, already must-use on Rust 1.99.
        #[allow(clippy::double_must_use)]
        pub trait Store: Send + Sync { $($methods)* }
    };
}
include!("store/contract/part_3.rs");
include!("store/contract/part_2.rs");
include!("store/contract/part_1.rs");
include!("store/contract/part_4.rs");
store_contract_0! {}
