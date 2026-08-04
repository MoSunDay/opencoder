//! Drain command channel.
//!
//! Operations like manual compaction, plan->act handoff, config hot-reload, and
//! live skill switching need `&mut SessionState`, which only exists inside the
//! background drain task. Rather than exposing the session directly, these
//! commands are queued onto an unbounded channel held by the
//! [`crate::handle::SessionHandle`]. The drain task drains the receiver after
//! each `run` completion and applies the commands in order.

#[derive(Debug)]
pub enum DrainCmd {
    Compact,
    Handoff { extra: String },
    SetSkill(Option<String>),
    ReloadConfig,
}
