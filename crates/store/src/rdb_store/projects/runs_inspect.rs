//! Bounded inspection and byte-chunked text reads over project TODO run
//! rows: text limits, summaries, version listing/paging, stale-run sweeps
//! and next-version lookup.

use anyhow::{Context, Result};

use crate::project_types::{ProjectRunText, ProjectTodoRunRecord, ProjectTodoRunStatus};
use crate::rdb_store::kv::{decode, get_json};
use crate::rdb_store::projects::keys::{run_index_prefix, run_key, run_root};
use crate::rdb_store::projects::runs::get;
use crate::rdb_store::projects::todos;
use crate::rdb_store::{seq, tx};
use crate::{PayloadChunkRecord, ProjectTodoRunPage, ProjectTodoRunSummary};

const TEXT_LIMIT: usize = 64 * 1024;
const PAGE_LIMIT: usize = 100;
const PAGE_BYTES: usize = 512 * 1024;

/// The bounded view of one text field: payloads within the chunk limit are
/// returned verbatim, larger ones are summarized by their byte total.
pub fn bounded_text(value: &str, field: String) -> ProjectRunText {
    let bytes = value.len();
    if bytes > TEXT_LIMIT {
        ProjectRunText::Omitted {
            omitted: true,
            total_bytes: bytes as u64,
            read_via: "detail_field",
            field,
        }
    } else {
        ProjectRunText::Text(value.to_owned())
    }
}

fn optional_text(value: Option<&String>, field: String) -> Option<ProjectRunText> {
    value.map(|v| bounded_text(v, field))
}

/// Does the todo already have a run in the `running` state?
pub fn has_running(store: &rdb::store::Store, todo_id: &str) -> Result<bool> {
    Ok(list(store, todo_id)?
        .iter()
        .any(|r| r.status == ProjectTodoRunStatus::Running))
}

pub fn get_summary(store: &rdb::store::Store, id: &str) -> Result<Option<ProjectTodoRunSummary>> {
    let Some(rec) = get(store, id)? else {
        return Ok(None);
    };
    Ok(Some(ProjectTodoRunSummary {
        input_snapshot: optional_text(
            rec.input_snapshot.as_ref(),
            format!("project.run.{id}.input_snapshot"),
        ),
        trace_manifest: optional_text(
            rec.trace_manifest.as_ref(),
            format!("project.run.{id}.trace_manifest"),
        ),
        id: rec.id,
        todo_id: rec.todo_id,
        kind: rec.kind,
        version: rec.version,
        plan_md: optional_text(rec.plan_md.as_ref(), format!("project.run.{id}.plan_md")),
        output_md: optional_text(
            rec.output_md.as_ref(),
            format!("project.run.{id}.output_md"),
        ),
        agent: rec.agent,
        executor_kind: rec.executor_kind,
        capability_id: rec.capability_id,
        plan_id: rec.plan_id,
        output_ref: rec.output_ref,
        session_id: rec.session_id,
        status: rec.status,
        started_at: rec.started_at,
        finished_at: rec.finished_at,
        created_at: rec.created_at,
    }))
}

/// Newest version first.
pub fn list(store: &rdb::store::Store, todo_id: &str) -> Result<Vec<ProjectTodoRunRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, &run_index_prefix(todo_id))? {
        let run_id =
            String::from_utf8(value).map_err(|_| anyhow::anyhow!("invalid run index owner"))?;
        let run = get_json::<ProjectTodoRunRecord>(store, &run_key(&run_id))?
            .with_context(|| format!("run index dangles at {}", String::from_utf8_lossy(&key)))?;
        rows.push(run);
    }
    rows.sort_by_key(|run| std::cmp::Reverse(run.version));
    Ok(rows)
}

/// Bounded page: `limit` clamped to `1..=100`, one lookahead row for
/// `next_version`, then a combined byte budget over the serialized page.
pub fn list_page(
    store: &rdb::store::Store,
    todo_id: &str,
    before_version: Option<i64>,
    limit: u32,
) -> Result<ProjectTodoRunPage> {
    let limit = limit.clamp(1, PAGE_LIMIT as u32) as usize;
    let mut rows: Vec<ProjectTodoRunSummary> = list(store, todo_id)?
        .into_iter()
        .filter(|run| before_version.is_none_or(|before| run.version < before))
        .take(limit + 1)
        .map(|run| run_summary(&run))
        .collect();
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    bound_page(rows, limit, has_more)
}

fn run_summary(rec: &ProjectTodoRunRecord) -> ProjectTodoRunSummary {
    ProjectTodoRunSummary {
        input_snapshot: optional_text(
            rec.input_snapshot.as_ref(),
            format!("project.run.{}.input_snapshot", rec.id),
        ),
        trace_manifest: optional_text(
            rec.trace_manifest.as_ref(),
            format!("project.run.{}.trace_manifest", rec.id),
        ),
        id: rec.id.clone(),
        todo_id: rec.todo_id.clone(),
        kind: rec.kind,
        version: rec.version,
        plan_md: optional_text(
            rec.plan_md.as_ref(),
            format!("project.run.{}.plan_md", rec.id),
        ),
        output_md: optional_text(
            rec.output_md.as_ref(),
            format!("project.run.{}.output_md", rec.id),
        ),
        agent: rec.agent.clone(),
        executor_kind: rec.executor_kind,
        capability_id: rec.capability_id.clone(),
        plan_id: rec.plan_id.clone(),
        output_ref: rec.output_ref.clone(),
        session_id: rec.session_id.clone(),
        status: rec.status,
        started_at: rec.started_at,
        finished_at: rec.finished_at,
        created_at: rec.created_at,
    }
}

/// Bound the page by item count AND serialized bytes (`512 KiB` total).
fn bound_page(
    mut runs: Vec<ProjectTodoRunSummary>,
    limit: usize,
    has_more: bool,
) -> Result<ProjectTodoRunPage> {
    let mut bytes = 0usize;
    let mut count = 0usize;
    for run in runs.iter().take(limit) {
        let size = serde_json::to_vec(run)?.len();
        if bytes + size > PAGE_BYTES && count > 0 {
            break;
        }
        bytes += size;
        count += 1;
    }
    let more = has_more || runs.len() > count;
    runs.truncate(count);
    Ok(ProjectTodoRunPage {
        next_version: more.then(|| runs.last().expect("nonempty when more").version),
        runs,
    })
}

/// Every run row currently in the `running` state (any todo, any kind) —
/// feeds the opportunistic stale-run sweep.
pub fn list_running(store: &rdb::store::Store) -> Result<Vec<ProjectTodoRunRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, &run_root())? {
        let run = decode::<ProjectTodoRunRecord>(&value)
            .with_context(|| format!("decode {}", String::from_utf8_lossy(&key)))?;
        if run.status == ProjectTodoRunStatus::Running {
            rows.push(run);
        }
    }
    Ok(rows)
}

/// `COALESCE(MAX(version), 0) + 1` for the todo — 1 for a fresh todo.
pub fn next_version(store: &rdb::store::Store, todo_id: &str) -> i64 {
    seq::max_child(store, &run_index_prefix(todo_id)) + 1
}

/// `substr`-equivalent byte window over one todo/run text field; NULL
/// fields read as `None`, an offset past the end is a caller error.
pub fn text_chunk(
    store: &rdb::store::Store,
    record_kind: &str,
    owner_id: &str,
    id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<PayloadChunkRecord>> {
    let selector = match (record_kind, field) {
        ("todo", "draft")
        | ("todo", "plan_md")
        | ("run", "plan_md")
        | ("run", "output_md")
        | ("run", "input_snapshot")
        | ("run", "trace_manifest") => (record_kind, field),
        _ => anyhow::bail!("unsupported project text field"),
    };
    let text: Option<String> = match selector {
        ("todo", field) => todos::get(store, id)?
            .filter(|rec| rec.id == owner_id)
            .and_then(|rec| match field {
                "draft" => Some(rec.draft),
                _ => rec.plan_md,
            }),
        _ => get(store, id)?
            .filter(|run| run.todo_id == owner_id)
            .and_then(|run| match field {
                "plan_md" => run.plan_md,
                "output_md" => run.output_md,
                "input_snapshot" => run.input_snapshot,
                _ => run.trace_manifest,
            }),
    };
    let Some(text) = text else {
        return Ok(None);
    };
    let bytes = text.as_bytes();
    let total = bytes.len() as u64;
    anyhow::ensure!(offset <= total, "project text offset exceeds total bytes");
    let take = max_bytes.clamp(1, TEXT_LIMIT);
    let start = offset as usize;
    let end = start.saturating_add(take).min(bytes.len());
    Ok(Some(PayloadChunkRecord {
        total_bytes: total,
        bytes: bytes[start..end].to_vec(),
    }))
}
