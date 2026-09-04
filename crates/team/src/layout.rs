//! Pure path/validation helpers for the opencoder-team NFS layout.
//!
//! ```text
//! <team_root>/<team_name>/team.json                                  ← 团队元信息
//! <team_root>/<team_name>/<topic_id>/team.json                       ← 话题元信息
//! <team_root>/<team_name>/<topic_id>/<turn>/plan.json
//! <team_root>/<team_name>/<topic_id>/<turn>/<sub_turn>/<member>/result.json
//! <team_root>/<team_name>/<topic_id>/<turn>/<sub_turn>/summary.json
//! ```
//!
//! Every path constructor validates its segments FIRST (dag/artifacts.rs
//! style) so a hostile name can never traverse out of `team_root`. The only
//! IO here are the two `list_*_dirs` directory scans.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Turns are 1-based, sub-turns 0-based, both capped at three digits so a
/// directory listing can never be flooded by runaway counters.
pub const MAX_TURN: usize = 999;
pub const MAX_SUB_TURN: usize = 999;

/// `^[a-z0-9][a-z0-9-]{0,63}$` — team directory name.
pub fn validate_team_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    let rest: Vec<char> = chars.collect();
    rest.len() <= 63
        && rest
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
}

/// Topic ids are ULIDs (they also key `team_topic_runs` rows).
pub fn validate_topic_id(id: &str) -> bool {
    ulid::Ulid::from_string(id).is_ok()
}

pub fn validate_turn(turn: usize) -> bool {
    (1..=MAX_TURN).contains(&turn)
}

pub fn validate_sub_turn(sub: usize) -> bool {
    sub <= MAX_SUB_TURN
}

/// Member ids are node ids: non-empty, ≤64 chars, `[A-Za-z0-9_-]` (same rule
/// as dag/artifacts `validate_run_id`). ULIDs always pass.
pub fn validate_member(node_id: &str) -> bool {
    !node_id.is_empty()
        && node_id.len() <= 64
        && node_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn checked(name: &str, ok: bool, what: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(anyhow!("illegal {what} {name:?}"))
    }
}

fn seg(root: &Path, name: &str, ok: bool, what: &str) -> Result<PathBuf> {
    checked(name, ok, what)?;
    Ok(root.join(name))
}

/// `<team_root>/<team_name>`
pub fn team_dir(root: &Path, team_name: &str) -> Result<PathBuf> {
    seg(root, team_name, validate_team_name(team_name), "team name")
}

/// `<team_root>/<team_name>/team.json`
pub fn team_file(root: &Path, team_name: &str) -> Result<PathBuf> {
    Ok(team_dir(root, team_name)?.join("team.json"))
}

/// `<team_root>/<team_name>/<topic_id>`
pub fn topic_dir(root: &Path, team_name: &str, topic_id: &str) -> Result<PathBuf> {
    let dir = team_dir(root, team_name)?;
    seg(&dir, topic_id, validate_topic_id(topic_id), "topic id")
}

/// `<team_root>/<team_name>/<topic_id>/team.json` (topic metadata file name
/// is deliberately `team.json` too — one reader serves both levels).
pub fn topic_file(root: &Path, team_name: &str, topic_id: &str) -> Result<PathBuf> {
    Ok(topic_dir(root, team_name, topic_id)?.join("team.json"))
}

/// `.../<topic_id>/<turn>`
pub fn turn_dir(root: &Path, team_name: &str, topic_id: &str, turn: usize) -> Result<PathBuf> {
    let dir = topic_dir(root, team_name, topic_id)?;
    seg(&dir, &turn.to_string(), validate_turn(turn), "turn")
}

/// `.../<topic_id>/<turn>/plan.json`
pub fn plan_file(root: &Path, team_name: &str, topic_id: &str, turn: usize) -> Result<PathBuf> {
    Ok(turn_dir(root, team_name, topic_id, turn)?.join("plan.json"))
}

/// `.../<topic_id>/<turn>/<sub_turn>`
pub fn sub_dir(
    root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
) -> Result<PathBuf> {
    let dir = turn_dir(root, team_name, topic_id, turn)?;
    seg(
        &dir,
        &sub_turn.to_string(),
        validate_sub_turn(sub_turn),
        "sub turn",
    )
}

/// `.../<turn>/<sub_turn>/<member>/result.json` — the member owns a directory.
#[allow(clippy::too_many_arguments)]
pub fn result_file(
    root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
    member: &str,
) -> Result<PathBuf> {
    let dir = sub_dir(root, team_name, topic_id, turn, sub_turn)?;
    seg(&dir, member, validate_member(member), "member node id")?;
    Ok(dir.join(member).join("result.json"))
}

/// `.../<turn>/<sub_turn>/summary.json`
pub fn summary_file(
    root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
) -> Result<PathBuf> {
    Ok(sub_dir(root, team_name, topic_id, turn, sub_turn)?.join("summary.json"))
}

/// All valid team directory names under `root`, sorted. Pure listing: it does
/// NOT check for a readable `team.json` (that is `fs_store::list_teams`).
pub fn list_team_dirs(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if validate_team_name(&name) && entry.path().is_dir() {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// All valid ULID topic directory names inside a team dir, sorted.
pub fn list_topic_dirs(team_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !team_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(team_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if validate_topic_id(&name) && entry.path().is_dir() {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// All member node ids that own a result directory inside a sub-turn dir
/// (validated names only, sorted). Unknown/garbage dirs are invisible.
pub fn list_valid_members(sub_turn_dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !sub_turn_dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(sub_turn_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if validate_member(&name) && entry.path().is_dir() {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}
