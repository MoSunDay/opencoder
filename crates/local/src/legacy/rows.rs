//! Typed legacy rows and bundle assembly.
//!
//! The row structs mirror exactly the columns of the legacy SQLite session
//! store (`sessions` / `messages` / `session_events` / `session_inputs` /
//! `subagent_tasks`); obsolete columns never reach this layer. Assembly is a
//! pure function from a decoded dataset to the public
//! [`SessionBundle`](opencoder_store::SessionBundle) shape, so migration goes
//! through `import_bundle` instead of direct KV writes.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};

use opencoder_core::Message;
use opencoder_store::{
    SessionBundle, SessionEventRecord, SessionInput, SessionMeta, SubagentBundle,
    SubagentTaskRecord,
};

/// One `sessions` row (the columns that still exist in `SessionMeta`).
#[derive(Debug, Clone)]
pub struct LegacySession {
    pub meta: SessionMeta,
}

/// One `messages` row (`summary = 0` only; compaction rows are excluded
/// upstream by the extraction query).
#[derive(Debug, Clone)]
pub struct LegacyMessage {
    pub session_id: String,
    pub message: Message,
}

/// One `session_events` row.
#[derive(Debug, Clone)]
pub struct LegacyEvent {
    pub record: SessionEventRecord,
}

/// One `session_inputs` row.
#[derive(Debug, Clone)]
pub struct LegacyInput {
    pub input: SessionInput,
}

/// One `subagent_tasks` row.
#[derive(Debug, Clone)]
pub struct LegacyTask {
    pub task: SubagentTaskRecord,
}

/// Everything read from one legacy store. Tables absent from a very old
/// source are reported in `absent_tables` and contribute zero rows.
#[derive(Debug, Clone, Default)]
pub struct LegacyDataset {
    pub sessions: Vec<LegacySession>,
    pub messages: Vec<LegacyMessage>,
    pub events: Vec<LegacyEvent>,
    pub inputs: Vec<LegacyInput>,
    pub subagent_tasks: Vec<LegacyTask>,
    pub absent_tables: Vec<String>,
}

/// Source-side totals recorded in the migration marker and reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigrationCounts {
    pub roots: u64,
    pub sessions: u64,
    pub messages: u64,
    pub events: u64,
    pub inputs: u64,
    pub subagent_tasks: u64,
}

impl MigrationCounts {
    fn of(dataset: &LegacyDataset, roots: u64) -> Self {
        MigrationCounts {
            roots,
            sessions: dataset.sessions.len() as u64,
            messages: dataset.messages.len() as u64,
            events: dataset.events.len() as u64,
            inputs: dataset.inputs.len() as u64,
            subagent_tasks: dataset.subagent_tasks.len() as u64,
        }
    }
}

/// The assembled migration unit: root bundles in stable order plus totals.
#[derive(Debug, Clone, Default)]
pub struct SessionForest {
    pub roots: Vec<SessionBundle>,
    pub counts: MigrationCounts,
}

/// Mirrors the importer's cycle guard: subagent nesting deeper than this is
/// treated as a referential cycle and rejected instead of recursing forever.
const MAX_DEPTH: usize = 32;

/// Group rows by session while preserving the extraction order (SQL `seq`
/// order) inside each session.
fn group_by_session<T>(items: impl IntoIterator<Item = (String, T)>) -> BTreeMap<String, Vec<T>> {
    let mut grouped = BTreeMap::<String, Vec<T>>::new();
    for (session_id, item) in items {
        grouped.entry(session_id).or_default().push(item);
    }
    grouped
}

/// Assemble root bundles from a decoded dataset.
///
/// A session is a root when no `subagent_tasks` row references it as its
/// child; roots are emitted ordered by `created_at`, then id, so repeated
/// runs report the same order. Dangling references (a task whose parent or
/// child session row is missing) are hard errors — rows are never silently
/// dropped.
pub fn assemble_roots(dataset: LegacyDataset) -> Result<SessionForest> {
    let mut ctx = AssemblyCtx {
        sessions: dataset
            .sessions
            .iter()
            .map(|s| (s.meta.id.clone(), s.meta.clone()))
            .collect(),
        messages: group_by_session(
            dataset
                .messages
                .iter()
                .map(|m| (m.session_id.clone(), m.message.clone())),
        ),
        events: group_by_session(
            dataset
                .events
                .iter()
                .map(|e| (e.record.session_id.clone(), e.record.clone())),
        ),
        inputs: group_by_session(
            dataset
                .inputs
                .iter()
                .map(|i| (i.input.session_id.clone(), i.input.clone())),
        ),
        tasks_by_parent: group_by_session(
            dataset
                .subagent_tasks
                .iter()
                .map(|t| (t.task.parent_session_id.clone(), t.task.clone())),
        ),
        visited: BTreeSet::new(),
    };

    let child_ids: BTreeSet<&str> = dataset
        .subagent_tasks
        .iter()
        .map(|t| t.task.child_session_id.as_str())
        .collect();
    // Dangling parent references must fail loudly: without this check the
    // orphaned child would be filtered out of the root set and silently lost.
    for task in &dataset.subagent_tasks {
        if !ctx.sessions.contains_key(&task.task.parent_session_id) {
            bail!(
                "legacy subagent task {} references missing parent session {}",
                task.task.task_id,
                task.task.parent_session_id
            );
        }
    }
    let mut root_ids: Vec<String> = ctx
        .sessions
        .keys()
        .filter(|id| !child_ids.contains(id.as_str()))
        .cloned()
        .collect();
    root_ids.sort_by_key(|id| {
        let meta = &ctx.sessions[id.as_str()];
        (meta.created_at, meta.id.clone())
    });

    let mut roots = Vec::with_capacity(root_ids.len());
    for id in &root_ids {
        roots.push(build_bundle(&mut ctx, id, 0)?);
    }
    // Coverage guard: a session reachable from no root (e.g. subagent cycles)
    // must fail the migration instead of being silently dropped.
    let unreachable: Vec<&str> = ctx
        .sessions
        .keys()
        .map(|id| id.as_str())
        .filter(|id| !ctx.visited.contains(*id))
        .collect();
    if !unreachable.is_empty() {
        bail!(
            "legacy sessions unreachable from any root (cyclic subagent_tasks?): {}",
            unreachable.join(", ")
        );
    }
    let counts = MigrationCounts::of(&dataset, roots.len() as u64);
    Ok(SessionForest { roots, counts })
}

/// Everything `build_bundle` needs; one struct keeps the recursion readable.
struct AssemblyCtx {
    sessions: BTreeMap<String, SessionMeta>,
    messages: BTreeMap<String, Vec<Message>>,
    events: BTreeMap<String, Vec<SessionEventRecord>>,
    inputs: BTreeMap<String, Vec<SessionInput>>,
    tasks_by_parent: BTreeMap<String, Vec<SubagentTaskRecord>>,
    /// Sessions already emitted into some bundle (cycle/coverage guard).
    visited: BTreeSet<String>,
}

fn build_bundle(ctx: &mut AssemblyCtx, session_id: &str, depth: usize) -> Result<SessionBundle> {
    if depth > MAX_DEPTH {
        bail!(
            "legacy subagent nesting exceeds max depth {MAX_DEPTH} at session \
             {session_id} (cyclic subagent_tasks references?)"
        );
    }
    let meta = ctx
        .sessions
        .get(session_id)
        .ok_or_else(|| {
            anyhow::anyhow!("legacy subagent_tasks references missing session {session_id}")
        })?
        .clone();
    let mut subagents = Vec::new();
    let tasks = ctx
        .tasks_by_parent
        .get(session_id)
        .cloned()
        .unwrap_or_default();
    for task in tasks {
        if !ctx.sessions.contains_key(&task.child_session_id) {
            bail!(
                "legacy subagent task {} references missing child session {}",
                task.task_id,
                task.child_session_id
            );
        }
        let child = build_bundle(ctx, &task.child_session_id, depth + 1)?;
        subagents.push(SubagentBundle { task, child });
    }
    ctx.visited.insert(session_id.to_string());
    Ok(SessionBundle {
        meta,
        messages: ctx.messages.get(session_id).cloned().unwrap_or_default(),
        events: ctx.events.get(session_id).cloned().unwrap_or_default(),
        inputs: ctx.inputs.get(session_id).cloned().unwrap_or_default(),
        subagents,
    })
}
