//! kb-index — deterministic knowledge-base index step for the code-review
//! release gate DAG (M2).
//!
//! Zero LLM calls: the module only walks the read-only knowledge mount and
//! writes its artifacts, so its output is fully reproducible.
//!
//! Contract with the DAG node runtime:
//! - `OPENCODER_KNOWLEDGE_DIR` — the read-only knowledge mount (default
//!   `/workspace/knowledge`, the runc bind path; the in-process runtime
//!   preopens the same guest path).
//! - `OPENCODER_STEP_DIR` — this step's artifact directory (default
//!   `/workspace/context/kb-index`); the runtime passes it with a trailing
//!   slash, which is trimmed.
//!
//! Artifacts:
//! - `<step_dir>/manifest.json` — a JSON array of
//!   `{"path": "<relative path>", "size": <bytes>}`, sorted by path
//!   (byte order) for determinism. Capped: at most [`MAX_ENTRIES`] entries
//!   and [`MAX_MANIFEST_BYTES`] serialized bytes; when a cap trips the walk
//!   stops and `truncated` is reported as `true`.
//! - `<step_dir>/output.json` — the structured step output consumed by the
//!   downstream agent steps.
//!
//! Read-only evidence: the module attempts to write `<knowledge>/.ro-probe`
//! (and to remove it again). On a read-only mount the write MUST fail —
//! that failure is the expected proof, recorded in `read_only_probe` and
//! never fatal. Every other IO error is fatal: message on stderr, exit 1.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Upper bound on manifest entries (whole-tree safety valve).
const MAX_ENTRIES: usize = 20_000;
/// Upper bound on the serialized manifest size, in bytes.
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const DEFAULT_KNOWLEDGE_DIR: &str = "/workspace/knowledge";
const DEFAULT_STEP_DIR: &str = "/workspace/context/kb-index";
const PROBE_NAME: &str = ".ro-probe";

struct Probe {
    write_attempted: bool,
    write_failed: bool,
    error: String,
}

struct Index {
    entries: Vec<(String, u64)>,
    /// Serialized manifest byte count for the entries kept so far.
    manifest_bytes: usize,
    truncated: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("kb-index: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let knowledge =
        env::var("OPENCODER_KNOWLEDGE_DIR").unwrap_or_else(|_| DEFAULT_KNOWLEDGE_DIR.to_string());
    let step_dir = env::var("OPENCODER_STEP_DIR").unwrap_or_else(|_| DEFAULT_STEP_DIR.to_string());
    let knowledge = PathBuf::from(knowledge.trim_end_matches('/'));
    let step_dir = PathBuf::from(step_dir.trim_end_matches('/'));

    if !knowledge.is_dir() {
        return Err(format!(
            "knowledge dir {} is missing or not a directory",
            knowledge.display()
        ));
    }
    fs::create_dir_all(&step_dir)
        .map_err(|e| format!("create step dir {}: {e}", step_dir.display()))?;

    let probe = read_only_probe(&knowledge);
    let mut index = Index {
        entries: Vec::new(),
        manifest_bytes: 0,
        truncated: false,
    };
    walk(&knowledge, &knowledge, &mut index)?;
    index.entries.sort_by(|a, b| a.0.cmp(&b.0));

    write_manifest(&step_dir, &index)?;
    write_output(&step_dir, &knowledge, &probe, &index)?;
    write_summary(&knowledge, &probe, &index);
    Ok(())
}

/// Recursively collect `(relative_path, size)` pairs. Deterministic: the
/// walk is stack-based and the result is sorted afterwards; symlinks are
/// skipped (never followed, never listed) so a poisoned link cannot make
/// the index depend on the host outside the mount.
fn walk(root: &Path, dir: &Path, index: &mut Index) -> Result<(), String> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read =
            fs::read_dir(&current).map_err(|e| format!("read dir {}: {e}", current.display()))?;
        let mut children: Vec<PathBuf> = Vec::new();
        for entry in read {
            let entry = entry.map_err(|e| format!("read entry in {}: {e}", current.display()))?;
            let meta = fs::symlink_metadata(entry.path())
                .map_err(|e| format!("stat {}: {e}", entry.path().display()))?;
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                children.push(entry.path());
            } else if file_type.is_file() {
                let path = entry
                    .path()
                    .strip_prefix(root)
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if !path.is_empty() {
                    try_push(index, &path, meta.len())?;
                }
            }
        }
        // Deepest-last stack order keeps the walk itself stable; the final
        // sort is what pins determinism.
        children.sort();
        stack.extend(children);
    }
    Ok(())
}

/// Record one file entry unless a cap is already tripped. Returns whether
/// the entry was accepted. Once truncated, no further entry is considered.
fn try_push(index: &mut Index, path: &str, size: u64) -> Result<bool, String> {
    if index.truncated {
        return Ok(false);
    }
    if index.entries.len() >= MAX_ENTRIES {
        index.truncated = true;
        return Ok(false);
    }
    let serialized = entry_json(path, size).len();
    let separator = if index.entries.is_empty() { 0 } else { 1 };
    if index.manifest_bytes + separator + serialized > MAX_MANIFEST_BYTES {
        index.truncated = true;
        return Ok(false);
    }
    index.manifest_bytes += separator + serialized;
    index.entries.push((path.to_string(), size));
    Ok(true)
}

/// `{"path":"...","size":N}` with the path JSON-escaped.
fn entry_json(path: &str, size: u64) -> String {
    format!("{{\"path\":{},\"size\":{}}}", json_string(path), size)
}

/// Minimal JSON string encoder (quotes + backslash + control chars).
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Write `<step_dir>/manifest.json`: the entries array (LOCKED shape).
fn write_manifest(step_dir: &Path, index: &Index) -> Result<(), String> {
    let mut body = String::from("[");
    for (position, (path, size)) in index.entries.iter().enumerate() {
        if position > 0 {
            body.push(',');
        }
        body.push_str(&entry_json(path, *size));
    }
    body.push(']');
    atomic_write(&step_dir.join("manifest.json"), &body)
}

/// Write `<step_dir>/output.json`: the structured step output downstream
/// agent steps read through the run's context chain.
fn write_output(
    step_dir: &Path,
    knowledge: &Path,
    probe: &Probe,
    index: &Index,
) -> Result<(), String> {
    let body = format!(
        "{{\"entries\":{},\"truncated\":{},\"manifest\":\"manifest.json\",\"knowledge_root\":{},\"read_only_probe\":{{\"write_attempted\":{},\"write_failed\":{},\"error\":{}}}}}",
        index.entries.len(),
        index.truncated,
        json_string(&knowledge.to_string_lossy()),
        probe.write_attempted,
        probe.write_failed,
        json_string(&probe.error),
    );
    atomic_write(&step_dir.join("output.json"), &body)
}

/// One-line summary JSON on stdout (becomes the step's `output.txt` tail).
fn write_summary(knowledge: &Path, probe: &Probe, index: &Index) {
    let line = format!(
        "{{\"step\":\"kb-index\",\"entries\":{},\"truncated\":{},\"knowledge_root\":{},\"read_only_probe_write_failed\":{}}}",
        index.entries.len(),
        index.truncated,
        json_string(&knowledge.to_string_lossy()),
        probe.write_failed,
    );
    println!("{line}");
}

/// Attempt `<knowledge>/.ro-probe`: write then remove. On the read-only
/// mount the write fails — that failure is expected evidence, never fatal.
/// A failing removal after a successful write is recorded but not fatal
/// either (the mount is writable then; the file is best-effort cleanup).
fn read_only_probe(knowledge: &Path) -> Probe {
    let target = knowledge.join(PROBE_NAME);
    let mut probe = Probe {
        write_attempted: true,
        write_failed: false,
        error: String::new(),
    };
    match fs::write(&target, b"kb-index read-only probe\n") {
        Ok(()) => {
            if let Err(e) = fs::remove_file(&target) {
                probe.error = format!("probe remove failed: {e}");
            }
        }
        Err(e) => {
            probe.write_failed = true;
            probe.error = e.to_string();
        }
    }
    probe
}

/// Write `bytes` to `path` via a temp file + rename so a crashed step can
/// never leave a half-written artifact behind.
fn atomic_write(path: &Path, body: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        file.write_all(body.as_bytes())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}
