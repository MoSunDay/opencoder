//! Project-module key families and the JSON row codec.
//!
//! Physical layout (mirrors the SQL tables one row per KV pair; the value is
//! the record's JSON so status/kind strings round-trip byte-for-byte and an
//! unknown string fails closed on read):
//! - `pgl/<id>`                          project_goals row
//! - `pin/<id>`                          project_initiatives row
//! - `ptd/<id>`                          project_todos row (people embedded)
//! - `prn/<id>`                          project_todo_runs row
//! - `idx/prn/todo/<todo>/<ver:020>`     run membership: version → run id
//! - `psv/<scope>/<id>/<ver:020>`        project_summary_versions row
//! - `ptg/<id>`                          project_tags row
//! - `idx/ptg/scope/<type>/<sid>/<name>` UNIQUE(scope_type,scope_id,name)
//! - `ptl/<todo>/<tag>`                  project_todo_tags link
//! - `pex/<todo>/<exec>`                 project_todo_executions row
//! - `pcm/<todo>/<seq:020>`              project_todo_comments (+state) row
//! - `idx/pcm/req/<todo>/<user>/<req>`   UNIQUE(todo_id,submitted_by,request_id)
//!
//! Identifiers come from callers and may contain `/`, so text components
//! that are not ULID-style ids are escaped ([`esc`]) before entering a key;
//! row payloads always carry the raw data, keys only need to be
//! collision-free and prefix-deletable.

use crate::project::comments::{Comment, NewComment};
use crate::project::summary::SummaryScope;
use crate::rdb_store::kv::pad_ord;
use serde::{Deserialize, Serialize};

/// Escape `\` and `/` so a component cannot forge a key boundary.
pub fn esc(part: &str) -> String {
    part.replace('\\', "\\\\").replace('/', "\\/")
}

pub fn goal_key(id: &str) -> Vec<u8> {
    format!("pgl/{id}").into_bytes()
}

pub fn initiative_key(id: &str) -> Vec<u8> {
    format!("pin/{id}").into_bytes()
}

pub fn todo_key(id: &str) -> Vec<u8> {
    format!("ptd/{id}").into_bytes()
}

pub fn todo_prefix() -> Vec<u8> {
    b"ptd/".to_vec()
}

pub fn run_key(id: &str) -> Vec<u8> {
    format!("prn/{id}").into_bytes()
}

pub fn run_root() -> Vec<u8> {
    b"prn/".to_vec()
}

pub fn run_index_prefix(todo_id: &str) -> Vec<u8> {
    format!("idx/prn/todo/{}/", esc(todo_id)).into_bytes()
}

pub fn run_index_key(todo_id: &str, version: i64) -> Vec<u8> {
    let mut k = run_index_prefix(todo_id);
    k.extend_from_slice(pad_ord(version).as_bytes());
    k
}

pub fn summary_prefix(scope: SummaryScope, id: &str) -> Vec<u8> {
    format!("psv/{}/{}/", scope.as_str(), esc(id)).into_bytes()
}

pub fn tag_key(id: &str) -> Vec<u8> {
    format!("ptg/{}", esc(id)).into_bytes()
}

pub fn tag_root() -> Vec<u8> {
    b"ptg/".to_vec()
}

/// Secondary key of `UNIQUE(scope_type, scope_id, name)` on project_tags.
pub fn tag_scope_index(scope_type: &str, scope_id: &str, name: &str) -> Vec<u8> {
    format!(
        "idx/ptg/scope/{}/{}/{}",
        esc(scope_type),
        esc(scope_id),
        esc(name)
    )
    .into_bytes()
}

pub fn todo_tag_key(todo_id: &str, tag_id: &str) -> Vec<u8> {
    format!("ptl/{}/{}", esc(todo_id), esc(tag_id)).into_bytes()
}

pub fn todo_tag_prefix(todo_id: &str) -> Vec<u8> {
    format!("ptl/{}/", esc(todo_id)).into_bytes()
}

pub fn todo_tag_root() -> Vec<u8> {
    b"ptl/".to_vec()
}

pub fn execution_key(todo_id: &str, execution_id: &str) -> Vec<u8> {
    format!("pex/{}/{}", esc(todo_id), esc(execution_id)).into_bytes()
}

pub fn execution_prefix(todo_id: &str) -> Vec<u8> {
    format!("pex/{}/", esc(todo_id)).into_bytes()
}

pub fn execution_root() -> Vec<u8> {
    b"pex/".to_vec()
}

pub fn comment_prefix(todo_id: &str) -> Vec<u8> {
    format!("pcm/{}/", esc(todo_id)).into_bytes()
}

pub fn comment_key(todo_id: &str, seq: i64) -> Vec<u8> {
    let mut k = comment_prefix(todo_id);
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

/// Prefix of the request-id index for one todo (cascade delete scope).
pub fn comment_request_prefix(todo_id: &str) -> Vec<u8> {
    format!("idx/pcm/req/{}/", esc(todo_id)).into_bytes()
}

/// Secondary key of `UNIQUE(todo_id, submitted_by, request_id)` on
/// project_todo_comments: value is the owning comment's seq.
pub fn comment_request_index(todo_id: &str, submitted_by: &str, request_id: &str) -> Vec<u8> {
    format!(
        "idx/pcm/req/{}/{}/{}",
        esc(todo_id),
        esc(submitted_by),
        esc(request_id)
    )
    .into_bytes()
}

/// One comment row: the appended entry merged with the mutation state
/// (`project_todo_comments` LEFT JOIN `project_todo_comment_state` in the
/// SQL backend; a deleted row keeps the state so old append requests cannot
/// resurrect it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentRow {
    #[serde(flatten)]
    pub entry: NewComment,
    pub revision: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_body_md: Option<String>,
    #[serde(default)]
    pub deleted: bool,
}

impl CommentRow {
    pub fn new(entry: &NewComment) -> Self {
        CommentRow {
            entry: entry.clone(),
            revision: 1,
            updated_at: entry.created_at,
            original_body_md: None,
            deleted: false,
        }
    }

    /// Projection the API returns (`original_body_md` is `#[serde(skip)]`).
    pub fn to_comment(&self, seq: i64) -> Comment {
        Comment {
            seq,
            revision: self.revision,
            updated_at: self.updated_at,
            original_body_md: self.original_body_md.clone(),
            entry: self.entry.clone(),
        }
    }
}
