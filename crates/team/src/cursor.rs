//! Resume cursor: the ENTIRE execution position of a topic is derived from
//! the NFS layout on every step, so `run_topic` is idempotent and resume is
//! just "run again" — no in-memory state survives a crash.
//!
//! For the active turn `T` (= max turn with a `plan.json`; `len(turns)+1`
//! when none exists yet):
//! - turn recorded in `meta.turns`  → only the closing decision is missing;
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
    let sub = missing.unwrap_or(max_sub_turns);
    Ok(Cursor {
        turn,
        stage: Stage::Sub { sub },
    })
}
