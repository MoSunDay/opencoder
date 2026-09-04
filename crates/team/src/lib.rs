//! # opencode-team
//!
//! Multi-agent team-discussion orchestration runtime. A **team** (captain +
//! members, each backed by a registered worker node) discusses a **topic**
//! over rounds:
//!
//! 1. **plan** (decision ①) — the captain picks the round's question and
//!    participating members;
//! 2. **sub-turns** — members answer (sub 0) or clarify ambiguities
//!    (sub ≥ 1, only for members the captain flagged); decision ② summarizes
//!    and judges alignment after every sub-turn;
//! 3. **closing** (decision ③) — the captain completes the topic or opens
//!    another round, bounded by `max_turns` / `max_sub_turns`.
//!
//! All durable state lives on an NFS-shared `team_root` directory tree
//! (atomic tmp+rename writes, see [`fs_store`]); execution position is fully
//! derived from that tree ([`cursor`]), so **`run_topic` is resume**: a
//! crashed/errored topic continues where its files stop. The
//! `(topic, node)` ledger in the store's `team_topic_runs` table tracks which
//! nodes are (or were) working which topic. Capability profiling
//! ([`profile_team`]) fills each member's `capabilities` for better planning.

pub mod config;
pub mod cursor;
pub mod decide;
pub mod dispatcher;
pub mod fs_store;
pub mod layout;
pub mod profile;
pub mod prompts;
pub mod runtime;
pub mod terminal;
pub mod types;

pub use config::{CancelToken, TeamRunConfig};
pub use cursor::{Cursor, Stage};
pub use dispatcher::{err, ok, DispatchCall, MockDispatcher, NodeDispatcher, TeamDispatcher};
pub use fs_store::{
    read_result, read_summary, read_topic_tree, read_turn_plan, save_team, save_topic, write_plan,
    write_result, write_summary, SubTurnView, TurnView, MAX_FILE_BYTES, MIN_FILE_BYTES,
};
pub use layout::{
    list_team_dirs, list_topic_dirs, list_valid_members, validate_member, validate_sub_turn,
    validate_team_name, validate_topic_id, validate_turn,
};
pub use profile::profile_team;
pub use runtime::{run_topic, start_topic};
pub use types::{
    parse_decision, Ambiguity, ClosingDecision, MemberRef, PlanDecision, PlanRecord,
    ProfileDecision, ResultRecord, SummaryDecision, SummaryRecord, TeamMember, TeamMeta, TopicMeta,
    TopicTurnMeta, FINISH_CANCELLED, FINISH_COMPLETE, FINISH_ERROR, FINISH_MAX_SUB_TURNS,
    FINISH_MAX_TURNS, RESULT_ALIGNMENT, RESULT_ANSWER, TOPIC_EXECUTING, TOPIC_FINISHED,
};
