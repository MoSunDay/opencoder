//! Read persisted results without starting or changing their owning Runtime.
//! Both the stable Host and standalone Worker use this producer.
mod snapshot;
#[cfg(test)]
mod tests;

use crate::DirectoryLayout;
use anyhow::{ensure, Context, Result};
use base64::Engine;
use opencoder_core::fleet::*;
use serde::Deserialize;
use serde_json::{json, Value};
use snapshot::{Snapshot, Stamp};
use std::{
    collections::VecDeque,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Default, Clone)]
pub struct ResultReader(
    Arc<Mutex<VecDeque<Entry>>>,
    #[cfg(test)] Arc<std::sync::atomic::AtomicUsize>,
);

struct Entry {
    path: PathBuf,
    stamp: Stamp,
    node: String,
    request: DetailFieldRequest,
    source: Option<(PathBuf, Stamp)>,
    snapshot: Snapshot,
}

#[derive(Deserialize)]
struct SavedRecord {
    assignment: SavedAssignment,
    result: Value,
}
#[derive(Deserialize)]
struct SavedAssignment {
    index: ExecutionIndex,
    request: SavedRequest,
    definition: Option<Value>,
}
#[derive(Deserialize)]
struct SavedRequest {
    id: String,
    kind: ExecutionKind,
}

impl ResultReader {
    pub fn supports(request: &DetailFieldRequest) -> bool {
        matches!(request.field.as_str(), "result" | "team.topic")
    }

    pub async fn read(
        &self,
        root: PathBuf,
        node: String,
        request: DetailFieldRequest,
    ) -> Result<RpcReply> {
        let reader = self.clone();
        tokio::task::spawn_blocking(move || reader.read_blocking(root, node, request)).await?
    }

    fn read_blocking(
        &self,
        root: PathBuf,
        node: String,
        request: DetailFieldRequest,
    ) -> Result<RpcReply> {
        ensure!(Self::supports(&request), "unsupported result field");
        let layout = DirectoryLayout::new(root, None)?;
        let current = layout.record_path(request.execution.kind, &request.execution.id)?;
        let legacy = !current.exists();
        let path = if legacy {
            layout.legacy_record_path(&request.execution.id)?
        } else {
            current
        };
        if path.exists() {
            snapshot::check_path(&path)?;
        }
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RpcReply::error(404, "execution result not found"))
            }
            Err(error) => return Err(error.into()),
        };
        let stamp = Stamp::of(&file.metadata()?);
        let mut cache = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("result reader lock poisoned"))?;
        let hit = cache.iter().position(|entry| {
            entry.path == path
                && entry.node == node
                && entry.request.execution == request.execution
                && entry.request.field == request.field
        });
        let mut entry = match hit.map(|i| cache.remove(i).unwrap()) {
            Some(entry) if entry.stamp == stamp && source_current(&entry)? => entry,
            _ => {
                #[cfg(test)]
                self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let saved: SavedRecord = serde_json::from_reader(std::io::BufReader::new(&file))?;
                let assignment = &saved.assignment;
                ensure!(
                    assignment.index.id == request.execution.id
                        && assignment.request.id == request.execution.id
                        && assignment.index.kind == request.execution.kind
                        && assignment.request.kind == request.execution.kind
                        && assignment.index.node_id == node,
                    "result execution identity mismatch"
                );
                let (source, snapshot) = if request.field == "result" {
                    (None, Snapshot::json(&saved.result)?)
                } else {
                    ensure!(
                        matches!(
                            request.execution.kind,
                            ExecutionKind::Team | ExecutionKind::System
                        ),
                        "topic requires a Team execution"
                    );
                    let name = if request.execution.kind == ExecutionKind::System {
                        "system"
                    } else {
                        assignment
                            .definition
                            .as_ref()
                            .and_then(|d| d["name"].as_str())
                            .context("team name missing")?
                    };
                    let root = if legacy {
                        layout.legacy_team_dir(&request.execution.id)?
                    } else {
                        layout.team_state_dir(request.execution.kind, &request.execution.id)?
                    };
                    let source =
                        opencoder_team::layout::topic_file(&root, name, &request.execution.id)?;
                    snapshot::check_path(&source)?;
                    let input = File::open(&source)?;
                    let before = Stamp::of(&input.metadata()?);
                    let snapshot = Snapshot::file(input)?;
                    if Stamp::of(&std::fs::metadata(&source)?) != before || !snapshot.unchanged()? {
                        return Ok(RpcReply::error(
                            409,
                            "result changed while opening snapshot",
                        ));
                    }
                    (Some((source, before)), snapshot)
                };
                if Stamp::of(&file.metadata()?) != stamp
                    || Stamp::of(&std::fs::metadata(&path)?) != stamp
                {
                    return Ok(RpcReply::error(
                        409,
                        "execution changed while opening result",
                    ));
                }
                Entry {
                    path,
                    stamp,
                    node,
                    request: request.clone(),
                    source,
                    snapshot,
                }
            }
        };
        let bytes = entry.snapshot.chunk(request.offset)?;
        if !source_current(&entry)?
            || !entry.snapshot.unchanged()?
            || Stamp::of(&std::fs::metadata(&entry.path)?) != entry.stamp
        {
            return Ok(RpcReply::error(409, "result changed during chunk read"));
        }
        let next = request.offset + bytes.len() as u64;
        let body = json!({"field":request.field,"offset":request.offset,"next_offset":next,
            "total_bytes":entry.snapshot.size,"eof":next == entry.snapshot.size,
            "encoding":"json-base64","bytes_b64":base64::engine::general_purpose::STANDARD.encode(bytes),
            "version":entry.snapshot.version});
        cache.push_back(entry);
        // Bounded descriptors and anonymous result snapshots; eviction and
        // process exit close/unlink them without touching execution artifacts.
        while cache.len() > 16 {
            cache.pop_front();
        }
        Ok(RpcReply::ok(body))
    }
}

fn source_current(entry: &Entry) -> Result<bool> {
    match &entry.source {
        Some((path, stamp)) => {
            snapshot::check_path(path)?;
            Ok(Stamp::of(&std::fs::metadata(path)?) == *stamp)
        }
        None => Ok(true),
    }
}
