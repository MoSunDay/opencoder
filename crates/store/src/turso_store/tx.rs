//! Transactions run within an exclusive write lease on database workers.
//! The bounded Busy retry and transaction helpers live in the shared engine
//! layer so every database writer queues on cross-process write-lock
//! conflicts with the same backoff budget.
pub(super) use opencoder_database::busy::{is_busy_conflict, run_tx};
