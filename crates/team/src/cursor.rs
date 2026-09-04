//! Resume cursor: the ENTIRE execution position of a topic is derived from
//! the NFS layout on every step, so `run_topic` is idempotent and resume is
//! just "run again" — no in-memory state survives a crash.
//!
//! For the active turn `T` (= max turn with a `plan.json`; `len(turns)+1`
//! when none exists yet):
//! - turn recorded in `meta.turns`  → only the closing decision is missing;
//! - otherwise the **largest existing summary** is authoritative: if it is
//!   `aligned` the turn is already decided (a crash between the
//!   `summary.json` write and the `meta.turns` push left it unrecorded), so
//!   the next step folds it into the metadata (`Record`) — no member is
//!   ever re-dispatched into a phantom sub-turn;
//! - otherwise the smallest sub-turn without `summary.json` is the work
//!   frontier (its missing member results are re-dispatched); if every
//!   sub-turn already has a summary the largest one is re-evaluated (crash
//!   between `summary.json` and the `meta.turns` push).

use std::path::Path;

use anyhow::Result;

use crate::fs_store::{self, TurnView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// No plan for the next turn yet: ask the captain for decision ①.
    Plan,
    /// Fill missing member results of `sub`, then decision ② (unless the
    /// summary already exists — then only evaluate it).
    Sub { sub: usize },
    /// The largest existing summary is `aligned` but the turn is not yet
    /// recorded in `meta.turns` (crash between the `summary.json` write and
    /// the metadata push): fold it into the topic metadata, then closing.
    Record { sub: usize },
    /// Turn is recorded; decision ③ (closing) is pending.
    Closing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub turn: usize,
    pub stage: Stage,
}

pub fn derive(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turns_done: &[usize],
    max_sub_turns: usize,
) -> Result<Cursor> {
    let (_, views) = fs_store::read_topic_tree(team_root, team_name, topic_id)?;
    let planned: Vec<&TurnView> = views.iter().filter(|v| v.plan.is_some()).collect();
    let Some(active) = planned.last() else {
        return Ok(Cursor {
            turn: turns_done.len() + 1,
            stage: Stage::Plan,
        });
    };
    let turn = active.turn;
    if turns_done.contains(&turn) {
        return Ok(Cursor {
            turn,
            stage: Stage::Closing,
        });
    }
    let missing = (0..=max_sub_turns).find(|s| {
        !active
            .sub_turns
            .iter()
            .any(|v| v.sub_turn == *s && v.summary.is_some())
    });
    let frontier = missing.unwrap_or(max_sub_turns);
    // The largest existing summary is authoritative: if it says `aligned`
    // the turn is decided and only the metadata push is missing — folding
    // it (`Record`) must win over re-dispatching a phantom follow-up
    // sub-turn that the aligned summary's ambiguities would suggest.
    let stage = match active
        .sub_turns
        .iter()
        .filter_map(|v| v.summary.as_ref().map(|s| (v.sub_turn, s)))
        .max_by_key(|(sub, _)| *sub)
    {
        Some((sub, summary)) if summary.aligned => Stage::Record { sub },
        _ => Stage::Sub { sub: frontier },
    };
    Ok(Cursor { turn, stage })
}
