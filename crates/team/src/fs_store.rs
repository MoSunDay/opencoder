//! The ONLY file in the crate that touches team-layout disk IO. All writes
//! go through tmp+rename so a concurrent reader on another NFS client sees
//! either the old or the new file, never a torn one. Sizes are bounded so a
//! runaway model reply cannot exhaust the share.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use ulid::Ulid;

use crate::layout::{self, MAX_SUB_TURN, MAX_TURN};
use crate::types::{PlanRecord, ResultRecord, SummaryRecord, TeamMeta, TopicMeta, TOPIC_EXECUTING};

/// Lower bound: a JSON document is at least `{}` — anything smaller (an
/// empty file, a truncated write) is corrupt and rejected before rename.
pub const MIN_FILE_BYTES: usize = 2;
/// Upper bound for any single layout file (prompt/reply-sized, not logs).
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// Atomic write: `<dir>/.<name>.tmp-<ulid>` → fsync → rename over target.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.len() < MIN_FILE_BYTES || bytes.len() > MAX_FILE_BYTES {
        bail!(
            "refusing {}-byte write to {} (bounds {}..={})",
            bytes.len(),
            path.display(),
            MIN_FILE_BYTES,
            MAX_FILE_BYTES
        );
    }
    let parent = path
        .parent()
        .with_context(|| format!("no parent for {}", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .with_context(|| format!("no file name for {}", path.display()))?;
    let tmp = parent.join(format!(".{}.tmp-{}", name.to_string_lossy(), Ulid::new()));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()));
    }
    Ok(())
}

/// Read a bounded file; `Ok(None)` only for "not there".
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > MAX_FILE_BYTES {
                bail!(
                    "{} is {} bytes (max {})",
                    path.display(),
                    bytes.len(),
                    MAX_FILE_BYTES
                );
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match read_bytes(path)? {
        None => Ok(None),
        Some(bytes) => {
            let value = serde_json::from_slice(&bytes)
                .with_context(|| format!("malformed JSON in {}", path.display()))?;
            Ok(Some(value))
        }
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &bytes)
}

// ── teams ──────────────────────────────────────────────────────────────────

pub fn create_team(team_root: &Path, meta: &TeamMeta) -> Result<()> {
    let path = layout::team_file(team_root, &meta.name)?;
    if path.exists() {
        bail!("team {:?} already exists", meta.name);
    }
    write_json(&path, meta)
}

pub fn load_team(team_root: &Path, name: &str) -> Result<TeamMeta> {
    let path = layout::team_file(team_root, name)?;
    read_json(&path)?.with_context(|| format!("team {name:?} not found"))
}

pub fn save_team(team_root: &Path, meta: &TeamMeta) -> Result<()> {
    let path = layout::team_file(team_root, &meta.name)?;
    write_json(&path, meta)
}

/// Every team with a readable `team.json`; corrupt entries are skipped with
/// a warning (a half-written foreign dir must not break listing).
pub fn list_teams(team_root: &Path) -> Vec<TeamMeta> {
    let mut out = Vec::new();
    let names = layout::list_team_dirs(team_root).unwrap_or_default();
    for name in names {
        match load_team(team_root, &name) {
            Ok(meta) => out.push(meta),
            Err(error) => {
                tracing::warn!(team = %name, error = %format!("{error:#}"), "skipping unreadable team")
            }
        }
    }
    out
}

// ── topics ────────────────────────────────────────────────────────────────

/// Create a fresh topic: new ULID, `executing` metadata written atomically.
pub fn init_topic(
    team_root: &Path,
    team_name: &str,
    title: &str,
    requirement: &str,
    captain: crate::types::MemberRef,
    members: Vec<crate::types::MemberRef>,
    now_ms: i64,
) -> Result<TopicMeta> {
    let topic_id = Ulid::new().to_string();
    let meta = TopicMeta {
        topic_id: topic_id.clone(),
        team_name: team_name.to_string(),
        title: title.to_string(),
        requirement: requirement.to_string(),
        status: TOPIC_EXECUTING.to_string(),
        finish_reason: None,
        created_at: now_ms,
        finished_at: None,
        captain,
        members,
        turns: Vec::new(),
        final_summary: None,
    };
    save_topic(team_root, &meta)?;
    Ok(meta)
}

pub fn load_topic(team_root: &Path, team_name: &str, topic_id: &str) -> Result<TopicMeta> {
    let path = layout::topic_file(team_root, team_name, topic_id)?;
    read_json(&path)?.with_context(|| format!("topic {topic_id} not found"))
}

pub fn save_topic(team_root: &Path, meta: &TopicMeta) -> Result<()> {
    let path = layout::topic_file(team_root, &meta.team_name, &meta.topic_id)?;
    write_json(&path, meta)
}

// ── turn artifacts ─────────────────────────────────────────────────────────

pub fn read_turn_plan(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
) -> Result<Option<PlanRecord>> {
    read_json(&layout::plan_file(team_root, team_name, topic_id, turn)?)
}

pub fn write_plan(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    plan: &PlanRecord,
) -> Result<()> {
    write_json(
        &layout::plan_file(team_root, team_name, topic_id, plan.turn)?,
        plan,
    )
}

pub fn read_result(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
    member: &str,
) -> Result<Option<ResultRecord>> {
    read_json(&layout::result_file(
        team_root, team_name, topic_id, turn, sub_turn, member,
    )?)
}

pub fn write_result(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    rec: &ResultRecord,
) -> Result<()> {
    write_json(
        &layout::result_file(
            team_root,
            team_name,
            topic_id,
            rec.turn,
            rec.sub_turn,
            &rec.node_id,
        )?,
        rec,
    )
}

pub fn read_summary(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
) -> Result<Option<SummaryRecord>> {
    read_json(&layout::summary_file(
        team_root, team_name, topic_id, turn, sub_turn,
    )?)
}

pub fn write_summary(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
    summary: &SummaryRecord,
) -> Result<()> {
    write_json(
        &layout::summary_file(team_root, team_name, topic_id, turn, sub_turn)?,
        summary,
    )
}

// ── full topic tree (web detail endpoint shape) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTurnView {
    pub sub_turn: usize,
    pub results: Vec<ResultRecord>,
    pub summary: Option<SummaryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnView {
    pub turn: usize,
    pub plan: Option<PlanRecord>,
    pub sub_turns: Vec<SubTurnView>,
}

/// Read the complete topic state from disk: metadata + every turn view
/// (plans, all member results, summaries). Pure read, cursor/web friendly.
pub fn read_topic_tree(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
) -> Result<(TopicMeta, Vec<TurnView>)> {
    let meta = load_topic(team_root, team_name, topic_id)?;
    let topic = layout::topic_dir(team_root, team_name, topic_id)?;
    let mut turns = Vec::new();
    for name in numeric_dirs(&topic, MAX_TURN)? {
        let turn: usize = name.parse().expect("numeric_dirs validated");
        let dir = topic.join(name);
        let plan: Option<PlanRecord> = read_json(&dir.join("plan.json"))?;
        let mut sub_turns = Vec::new();
        for sub in numeric_dirs(&dir, MAX_SUB_TURN)? {
            let sub_turn: usize = sub.parse().expect("numeric_dirs validated");
            let sdir = dir.join(&sub);
            let mut results = Vec::new();
            for member in layout::list_valid_members(&sdir)? {
                let path = sdir.join(&member).join("result.json");
                if let Some(rec) = read_json::<ResultRecord>(&path)? {
                    results.push(rec);
                }
            }
            results.sort_by(|a, b| a.node_id.cmp(&b.node_id));
            let summary = read_json::<SummaryRecord>(&sdir.join("summary.json"))?;
            sub_turns.push(SubTurnView {
                sub_turn,
                results,
                summary,
            });
        }
        turns.push(TurnView {
            turn,
            plan,
            sub_turns,
        });
    }
    turns.sort_by_key(|t| t.turn);
    Ok((meta, turns))
}

/// Numeric sub-directory names of `dir` bounded by `max`, as strings.
fn numeric_dirs(dir: &Path, max: usize) -> Result<Vec<String>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.path().is_dir() {
            continue;
        }
        if let Ok(value) = name.parse::<usize>() {
            if value <= max {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}
