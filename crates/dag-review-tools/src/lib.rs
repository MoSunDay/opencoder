//! Pure orchestration logic for the code-review release-gate wasm
//! modules (`env_eob_up` / `env_baremetal_up` / `harness_runner`).
//!
//! The modules are deterministic coordinators: every heavy action (a
//! deploy, a harness run) is delegated to a node-whitelisted `dag.ops`
//! command through the `opencoder` host imports (see [`host`]), and the
//! module's own job is parsing op evidence and assembling
//! `<step_dir>/output.json` — all of which is plain, testable logic.
//!
//! Op evidence contract (written by the node runtime): the file
//! `<step_dir>/ops/<op_id>.log` starts with `# <op_id> exit=N|killed(...)`
//! headers followed by a `-- output --` line and the raw captured output.
//! The node runtime guarantees op ids match `[A-Za-z0-9_-]{1,64}`, so the
//! evidence path is always a single safe path segment.

pub mod host;

use std::path::{Path, PathBuf};

/// Mirror of the runtime's probe error codes (dag-runtime
/// `host_imports::probe`); a module must treat any negative value as
/// "not up" and fail closed.
pub const PROBE_ERR_URL: i32 = -1;
pub const PROBE_ERR_UNREACHABLE: i32 = -2;
pub const PROBE_ERR_STATUS: i32 = -3;
pub const PROBE_ERR_CANCELLED: i32 = -4;

/// Evidence file (relative to the step dir) for one op run.
pub fn evidence_rel_path(op_id: &str) -> String {
    format!("ops/{op_id}.log")
}

/// `OPENCODER_STEP_DIR` with the runtime's trailing slash trimmed.
pub fn step_dir_from_env() -> PathBuf {
    std::env::var("OPENCODER_STEP_DIR")
        .unwrap_or_default()
        .trim_end_matches('/')
        .into()
}

/// Structured view of one op evidence log.
#[derive(Debug, Clone, PartialEq)]
pub struct OpEvidence {
    /// Natural exit code when the op ran to completion.
    pub exit: Option<i32>,
    /// Why the runtime killed the op (timeout / overflow / cancel).
    pub killed: Option<String>,
    /// Raw captured output (already bounded by the runtime).
    pub output: String,
}

/// Parse an op evidence log: header lines starting with `#`, then the
/// `-- output --` marker, then the captured output verbatim. Tolerant of
/// a missing marker (treats everything as output) because modules must
/// keep reporting even when evidence is malformed.
pub fn parse_op_log(raw: &str) -> OpEvidence {
    let mut exit = None;
    let mut killed = None;
    let mut output_start = None;
    for (idx, line) in raw.lines().enumerate() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(code) = rest.split(' ').find_map(|tok| {
                tok.strip_prefix("exit=")
                    .and_then(|v| v.parse::<i32>().ok())
            }) {
                exit = Some(code);
            }
            if let Some(idx) = rest.find("killed(") {
                let token = &rest[idx..];
                killed = Some(token.split(' ').next().unwrap_or(token).to_string());
            }
        } else if line.trim() == "-- output --" {
            output_start = Some(idx + 1);
            break;
        }
    }
    let output = match output_start {
        Some(from) => raw.lines().skip(from).collect::<Vec<_>>().join("\n"),
        None => raw.to_string(),
    };
    OpEvidence {
        exit,
        killed,
        output,
    }
}

/// Last non-empty line of `output` that parses as a JSON object.
pub fn tail_json(output: &str) -> Option<serde_json::Value> {
    output
        .lines()
        .rev()
        .find_map(|line| {
            let t = line.trim();
            if t.is_empty() || !t.starts_with('{') {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(t).ok()
        })
        .filter(|v| v.is_object())
}

/// One harness stage parsed from an op's NDJSON output.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub name: String,
    /// `ok` / `fail` / anything else is treated as `fail` (fail-closed).
    pub status: String,
    pub detail: String,
}

/// Parse NDJSON stage lines `{"stage": "...", "status": "...",
/// "detail": "..."}`. Unparsable or object-less lines are skipped: the
/// harness contract keeps evidence human-readable noise around stages.
pub fn parse_stages(output: &str) -> Vec<Stage> {
    output
        .lines()
        .filter_map(|line| {
            let v = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
            let obj = v.as_object()?;
            let name = obj.get("stage")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            Some(Stage {
                name,
                status: obj
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("fail")
                    .to_string(),
                detail: obj
                    .get("detail")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// A harness run passes only when at least one stage ran and every stage
/// reported `ok` (fail-closed, matching the release gate).
pub fn stages_pass(stages: &[Stage]) -> bool {
    !stages.is_empty() && stages.iter().all(|s| s.status == "ok")
}

/// Failed stage names, in first-seen order.
pub fn failed_stages(stages: &[Stage]) -> Vec<String> {
    stages
        .iter()
        .filter(|s| s.status != "ok")
        .map(|s| s.name.clone())
        .collect()
}

/// Assemble a deploy step's `output.json` body.
pub fn deploy_output(
    op_exit: Option<i32>,
    op_killed: Option<&str>,
    endpoint: &str,
    probe_code: i32,
    op_id: &str,
) -> serde_json::Value {
    let up = op_exit == Some(0) && op_killed.is_none() && (200..300).contains(&probe_code);
    serde_json::json!({
        "status": if up { "up" } else { "down" },
        "endpoint": endpoint,
        "http": probe_code,
        "op_exit": op_exit,
        "op_killed": op_killed,
        "evidence": evidence_rel_path(op_id),
    })
}

/// Assemble a harness step's `output.json` body.
pub fn harness_output(
    op_exit: Option<i32>,
    op_killed: Option<&str>,
    stages: &[Stage],
    op_id: &str,
) -> serde_json::Value {
    let op_ok = op_exit == Some(0) && op_killed.is_none();
    let passed = op_ok && stages_pass(stages);
    serde_json::json!({
        "status": if passed { "passed" } else { "failed" },
        "stages": stages.len(),
        "failed": failed_stages(stages),
        "op_exit": op_exit,
        "op_killed": op_killed,
        "evidence": evidence_rel_path(op_id),
    })
}

/// Write `output.json` into the step dir via temp file + rename so a
/// crashed module can never leave a half-written artifact behind.
pub fn write_output_json(step_dir: &Path, value: &serde_json::Value) -> Result<(), String> {
    let target = step_dir.join("output.json");
    let tmp = step_dir.join("output.json.tmp");
    std::fs::write(&tmp, value.to_string()).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &target)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), target.display()))
}

/// Extract a ready endpoint from an op's tail JSON. Accepts
/// `{"endpoint": "http://host:port"}` or `{"host": ..., "port": ...}`.
pub fn endpoint_of(tail: &serde_json::Value) -> Option<String> {
    if let Some(ep) = tail.get("endpoint").and_then(|v| v.as_str()) {
        return Some(ep.trim_end_matches('/').to_string());
    }
    let host = tail.get("host").and_then(|v| v.as_str())?;
    let port = tail.get("port").and_then(|v| v.as_i64())?;
    Some(format!("http://{host}:{port}"))
}
