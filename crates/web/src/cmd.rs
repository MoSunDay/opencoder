//! Drain command channel.
//!
//! Operations like manual compaction, execution handoff, config hot-reload, and
//! live skill switching need `&mut SessionState`, which only exists inside the
//! background drain task. Rather than exposing the session directly, these
//! commands are queued onto an unbounded channel held by the
//! [`crate::handle::SessionHandle`]. The drain task drains the receiver after
//! each `run` completion and applies the commands in order.

#[derive(Debug)]
pub enum DrainCmd {
    Compact,
    Handoff {
        extra: String,
    },
    SetSkill(Option<String>),
    ReloadConfig,
    /// Session-scoped autopilot switch (TUI `/ap` parity). Applies to the
    /// live `SessionState` (`ap_mode_override` + config mode) and persists.
    SetApMode(opencoder_core::ApMode),
    /// Edit (or clear, on `None`/blank) the user requirement annotation on
    /// the live session and persist it (TUI `/requirement` parity).
    SetAnnotation(Option<String>),
}
