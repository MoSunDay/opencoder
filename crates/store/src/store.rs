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

/// Storage abstraction — the single seam that lets us swap libsql for another
/// Rust SQLite implementation later without touching upper layers.
///
/// Upper-layer code depends on `Arc<dyn Store>`; concrete impls live in
/// `libsql_store` (primary) and any future backend.
// Rust traits and trait implementations cannot be split into partial items.
// Assemble method groups first so async_trait sees one complete declaration.
macro_rules! store_contract_finish {
    ($($methods:tt)*) => {
        #[async_trait]
        pub trait Store: Send + Sync { $($methods)* }
    };
}
include!("store/contract/part_3.rs");
include!("store/contract/part_2.rs");
include!("store/contract/part_1.rs");
store_contract_0! {}
