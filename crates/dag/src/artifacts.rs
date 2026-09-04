//! Node-local artifact directory contract (pure path/slug/truncation
//! helpers — no IO; the runtime applies them).
//!
//! Layout under the configured `workflow_root` (default `/workflow`):
//!
//! ```text
//! /workflow/<run_id>/                 <- also the runc /workspace/context mount
//!   <step-slug>/output.json           <- machine-readable step output (optional)
//!   <step-slug>/output.txt            <- captured stdout / transcript tail
//!   <step-slug>/meta.json             <- runtime-written step metadata
//! ```
//!
//! The SERVER never touches these files: browsers only ever see truncated
//! snapshots carried inside `step_done` events. Slugs are validated before
//! any path is built so a hostile spec cannot traverse out of the run dir.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Cap for output text carried inside `step_done` event payloads (UI
/// preview only; full artifacts stay on the node).
pub const MAX_SNAPSHOT_BYTES: usize = 4 * 1024;

/// `[a-z0-9][a-z0-9-]{0,63}` — also the artifact directory name.
pub fn validate_step_slug(name: &str) -> bool {
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

/// Run ids appear in paths: ULIDs are fine, traversal is not.
pub fn validate_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= 64
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `/workflow/<run_id>`
pub fn run_root(workflow_root: &Path, run_id: &str) -> Result<PathBuf, String> {
    if !validate_run_id(run_id) {
        return Err(format!("illegal run id {run_id:?}"));
    }
    Ok(workflow_root.join(run_id))
}

/// `/workflow/<run_id>/<step>` — slug-gated.
pub fn step_dir(workflow_root: &Path, run_id: &str, step: &str) -> Result<PathBuf, String> {
    if !validate_step_slug(step) {
        return Err(format!("illegal step slug {step:?}"));
    }
    Ok(run_root(workflow_root, run_id)?.join(step))
}

/// The runc sandbox mounts THIS directory at `/workspace/context` (rw) with
/// the container rootfs readonly: step code reads upstream outputs at
/// `/workspace/context/<upstream>/output.json` and writes its own to
/// `/workspace/context/<self>/output.json`.
pub fn context_dir(workflow_root: &Path, run_id: &str) -> Result<PathBuf, String> {
    run_root(workflow_root, run_id)
}

/// Truncate an output snapshot to [`MAX_SNAPSHOT_BYTES`] on a char boundary,
/// marking the cut so the UI can say "truncated".
pub fn output_snapshot(text: &str) -> String {
    if text.len() <= MAX_SNAPSHOT_BYTES {
        return text.to_string();
    }
    let mut cut = MAX_SNAPSHOT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[truncated]", &text[..cut])
}

/// The runtime-written `meta.json` body for one step.
pub fn meta_value(
    step: &str,
    outcome: &str,
    started_at_ms: i64,
    finished_at_ms: i64,
    error: Option<&str>,
) -> Value {
    json!({
        "step": step,
        "outcome": outcome,
        "started_at_ms": started_at_ms,
        "finished_at_ms": finished_at_ms,
        "error": error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_rules() {
        for ok in ["a", "a1", "fetch-2", "0x", &"x".repeat(64)] {
            assert!(validate_step_slug(ok), "{ok:?}");
        }
        for bad in [
            "",
            "A",
            "-a",
            "a_b",
            "a b",
            "a/b",
            "../etc",
            &"x".repeat(65),
            "é",
        ] {
            assert!(!validate_step_slug(bad), "{bad:?}");
        }
    }

    #[test]
    fn run_id_rejects_traversal() {
        assert!(validate_run_id("01JARUN"));
        assert!(validate_run_id("run-1_x"));
        for bad in ["", "..", "a/b", "a b", &"x".repeat(65), "."] {
            assert!(!validate_run_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn paths_are_rooted_and_slug_gated() {
        let root = Path::new("/workflow");
        assert_eq!(run_root(root, "01R").unwrap(), Path::new("/workflow/01R"));
        assert_eq!(
            step_dir(root, "01R", "fetch").unwrap(),
            Path::new("/workflow/01R/fetch")
        );
        assert_eq!(
            context_dir(root, "01R").unwrap(),
            run_root(root, "01R").unwrap()
        );
        // Traversal attempts fail BEFORE any path is built.
        assert!(step_dir(root, "01R", "../escape").is_err());
        assert!(run_root(root, "../etc").is_err());
    }

    #[test]
    fn snapshot_truncates_on_char_boundary_with_marker() {
        let short = "hello";
        assert_eq!(output_snapshot(short), "hello");
        let long = "ä".repeat(MAX_SNAPSHOT_BYTES); // 2-byte chars
        let snap = output_snapshot(&long);
        assert!(snap.ends_with("[truncated]"));
        assert!(snap.len() <= MAX_SNAPSHOT_BYTES + "\n[truncated]".len() + 1);
    }

    #[test]
    fn meta_shape() {
        let v = meta_value("fetch", "done", 1, 2, None);
        assert_eq!(v["step"], "fetch");
        assert_eq!(v["outcome"], "done");
        assert_eq!(v["error"], serde_json::Value::Null);
    }
}
