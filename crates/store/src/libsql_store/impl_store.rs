//! The single `impl Store for LibsqlStore` block, split out of `mod.rs` to
//! keep that file within the size budget. Rust allows exactly one impl block
//! per trait+type, so every store method lives here; the SQL itself stays in
//! per-domain free functions in the sibling modules.

use anyhow::Result;
use async_trait::async_trait;

use super::{
    brain, dag, dag_events, events, inputs, messages, node_tasks, nodes, schedule, sessions,
    subagent_tasks, team_runs, todos, users, LibsqlStore,
};
use crate::store::Store;
use crate::types::{
    ClearNodeDialogs, ConvergedDagRun, DagDefRecord, DagEventRecord, DagRunRecord, Delivery,
    ImportReport, InputAdmission, MessageChunkPage, MessageRow, NodeRecord, NodeTaskRecord,
    NodeTaskStatus, SessionEventPage, SessionEventRecord, SessionFilter, SessionInput,
    SessionListItem, SessionMeta, SessionPatch, SubagentTaskRecord,
};
use crate::{
    BrainCapabilityDetail, BrainCapabilityRecord, BrainEngInputRecord, BrainVectorHit,
    BrainVectorWrite, ScheduleDefRecord, ScheduleRunRecord, TeamTopicRunRecord, TodoEventPage,
    TodoEventRecord, TodoItemRecord, TodoWorkflowDetail, TodoWorkflowRecord, TodoWorkflowSummary,
};

// Rust traits and trait implementations cannot be split into partial items.
// Assemble method groups first so async_trait sees one complete declaration.
macro_rules! store_implementation_finish {
    ($($methods:tt)*) => {
        #[async_trait]
        impl Store for LibsqlStore { $($methods)* }
    };
}
include!("impl_methods/part_3.rs");
include!("impl_methods/part_2.rs");
include!("impl_methods/part_1.rs");
store_implementation_0! {}
