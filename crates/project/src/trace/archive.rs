//! Append-only, fsynced run files. Paths exposed to readers are logical names.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Archive {
    pub root: PathBuf,
    pub cancel: CancellationToken,
    state: Arc<Mutex<State>>,
}
#[derive(Default)]
struct State {
    events: u64,
    calls: u64,
    error: Option<String>,
}

pub fn run_root(root: &Path, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        id.starts_with("prun-") && opencoder_core::fleet::valid_id(id),
        "invalid project run id"
    );
    Ok(root.join(id))
}

pub fn write_new(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.sync_all()?;
    File::open(path.parent().context("archive parent missing")?)?.sync_all()?;
    Ok(())
}

impl Archive {
    pub fn create(root: PathBuf, cancel: CancellationToken) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        File::open(&root)?.sync_all()?;
        File::open(root.parent().context("archive parent missing")?)?.sync_all()?;
        Ok(Self {
            root,
            cancel,
            state: Arc::new(Mutex::new(State::default())),
        })
    }
    pub fn check(&self) -> Result<()> {
        if let Some(error) = &self.state.lock().unwrap().error {
            bail!("project persistence: {error}");
        }
        Ok(())
    }
    pub fn fail(&self, error: impl std::fmt::Display) {
        self.state
            .lock()
            .unwrap()
            .error
            .get_or_insert_with(|| error.to_string());
        self.cancel.cancel();
    }
    pub fn event(&self, kind: &str, data: Value) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.events += 1;
        let seq = state.events;
        let result =
            write_new(&self.root.join(format!("event-{seq}.json")), &data).and_then(|()| {
                write_new(
                    &self.root.join(format!("event-{seq}.meta.json")),
                    &json!({
                        "seq":seq,"kind":kind,"ts":opencoder_core::message::now_ms()
                    }),
                )
            });
        if let Err(error) = &result {
            state.error.get_or_insert_with(|| error.to_string());
            self.cancel.cancel();
        }
        result
    }
    pub fn request(&self, body: &Value) -> Result<u64> {
        self.check()?;
        let mut state = self.state.lock().unwrap();
        state.calls += 1;
        let id = state.calls;
        write_new(&self.root.join(format!("request-{id}.json")), body)?;
        Ok(id)
    }
    pub fn counts(&self) -> (u64, u64) {
        let s = self.state.lock().unwrap();
        (s.events, s.calls)
    }
    pub fn response(&self, call: u64, event: &Value) -> Result<()> {
        let path = self.root.join(format!("response-{call}.jsonl"));
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}

pub fn file_chunk(
    root: &Path,
    name: &str,
    offset: u64,
    max: usize,
) -> Result<Option<opencoder_store::PayloadChunkRecord>> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
            && name != "."
            && name != "..",
        "invalid archive file"
    );
    let path = root.join(name);
    if !path.exists() {
        return Ok(None);
    }
    let resolved = path.canonicalize()?;
    anyhow::ensure!(
        resolved.starts_with(root.canonicalize()?),
        "archive file escaped run directory"
    );
    let mut file = File::open(resolved)?;
    let total = file.metadata()?.len();
    anyhow::ensure!(offset <= total, "archive offset exceeds total bytes");
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(max.min(64 * 1024) as u64)
        .read_to_end(&mut bytes)?;
    Ok(Some(opencoder_store::PayloadChunkRecord {
        total_bytes: total,
        bytes,
    }))
}
