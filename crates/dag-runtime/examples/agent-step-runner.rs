//! Container-side single-step agent session runner (`dag.agent_sandbox = runc`).
//!
//! The host executes `agent` steps by launching THIS binary inside a
//! read-only OCI container (`ArgvStyle::Direct`, argv
//! `["/usr/bin/agent-step-runner"]`); the whole session — LLM loop, tools,
//! artifacts — then runs confined to the container with only
//! `/workspace/context/<step>` (rw) and `/workspace/knowledge` (ro) exposed.
//!
//! Contract (env, injected by the host executor; see `exec::agent_runc`):
//! - `OPENCODER_STEP_PROMPT`  — container path of the step's prompt file
//!   (REQUIRED; missing = contract violation, exit code 2);
//! - `OPENCODER_STEP_DIR`     — the step artifact dir (writable mount);
//! - `OPENCODER_STEP_SESSION_ID` — host-preallocated session id (console
//!   attachability survives; a fresh ULID when absent);
//! - `OPENCODER_STEP_AGENT`   — executing agent name (default `act`);
//! - `OPENCODER_HOW_APPEND`   — optional workflow-declared how.md payload;
//! - `OPENAI_BASE_URL` / `OPENAI_API_KEY` / `OPENCODER_MODEL` — the host's
//!   resolved LLM endpoint, applied through `Config`'s env overlay.
//!
//! Artifacts written into the step dir (host-visible through the bind):
//! `session.json` (`running` → `done`/`error`), `transcript.txt`, and the
//! optional `output.json` recovered from the final assistant text with the
//! same extraction contract as the host path. Exit code: 0 success, 1 run
//! failure, 2 contract violation.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::{run as run_session, SessionEvent, SessionState};

/// Bounded transcript artifact: keep the LAST bytes on a char boundary.
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(code) => std::process::exit(code),
    }
}

/// Returns the process exit code (0/1/2) after doing all the work.
fn run() -> Result<(), i32> {
    // 1. Step contract: the prompt file must exist before anything else.
    let prompt_path = match std::env::var("OPENCODER_STEP_PROMPT") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            eprintln!("agent-step-runner: OPENCODER_STEP_PROMPT is required (prompt file path)");
            return Err(2);
        }
    };
    let prompt = match std::fs::read_to_string(&prompt_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "agent-step-runner: cannot read prompt file {}: {error}",
                prompt_path.display()
            );
            return Err(2);
        }
    };
    let step_dir = PathBuf::from(env_or("OPENCODER_STEP_DIR", "/workspace/context/step"));
    if let Err(error) = std::fs::create_dir_all(&step_dir) {
        eprintln!(
            "agent-step-runner: cannot create step dir {}: {error}",
            step_dir.display()
        );
        return Err(2);
    }
    let session_id = env_or("OPENCODER_STEP_SESSION_ID", &ulid::Ulid::new().to_string());
    let agent_name = env_or("OPENCODER_STEP_AGENT", "act");

    // 2. Config: the container carries no config files, so `/workspace`
    // discovery plus the host-injected env overlay defines the endpoint.
    let config = match opencoder_core::Config::load(std::path::Path::new("/workspace")) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("agent-step-runner: config load failed: {error}");
            return Err(2);
        }
    };
    let endpoint = match config.resolve_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("agent-step-runner: cannot resolve LLM endpoint: {error}");
            return Err(1);
        }
    };
    let client: Arc<dyn ChatStream> = match ChatClient::from_config(&config, &endpoint) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("agent-step-runner: cannot build LLM client: {error}");
            return Err(1);
        }
    };
    let agent = match opencoder_dag_runtime::exec::how_copy::load(&step_dir) {
        Ok(agent) => agent,
        Err(error) => {
            eprintln!("agent-step-runner: cannot load frozen agent/how.md: {error:#}");
            return Err(2);
        }
    };

    // 3. Session pinned to the step dir (writable mount); the env contract
    // mirrors the host path's passthrough pair set.
    let mut session = SessionState::new(
        session_id.clone(),
        agent,
        config.clone(),
        client,
        step_dir.clone(),
    );
    if let Ok(how_append) = std::env::var("OPENCODER_HOW_APPEND") {
        if !how_append.is_empty() {
            session
                .env_passthrough
                .push(("OPENCODER_HOW_APPEND".into(), how_append));
        }
    }
    if std::env::var("OPENCODER_KNOWLEDGE_DIR").is_ok_and(|v| !v.is_empty()) {
        session
            .env_passthrough
            .push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));
    }

    // 4. Publish the live pointer immediately (the host wrote a minimal
    // `session.json`; this richer body keeps the same `session_id` key).
    write_session_json(
        &step_dir,
        &session_id,
        &agent_name,
        config.model_id(),
        "running",
        None,
    );

    // 5. Run exactly one turn, keeping a bounded transcript tail.
    let file = std::fs::File::create(step_dir.join("events.ndjson")).map_err(|e| {
        eprintln!("agent-step-runner: cannot create event stream: {e}");
        2
    })?;
    let events = Arc::new(Mutex::new(file));
    let event_error = Arc::new(Mutex::new(None));
    let failure = event_error.clone();
    let cancellation = tokio_util::sync::CancellationToken::new();
    session.cancel = Some(cancellation.clone());
    let transcript = Arc::new(Mutex::new(String::new()));
    let tail = Arc::clone(&transcript);
    let on_event = move |ev: SessionEvent| {
        if !ev.is_sidecar_frame() {
            let line =
                serde_json::json!({"kind":ev.sse_kind(),"payload":ev.sse_data()}).to_string();
            if let Err(error) = writeln!(events.lock().unwrap(), "{line}") {
                *failure.lock().unwrap() = Some(error.to_string());
                cancellation.cancel();
            }
        }
        if let SessionEvent::TextDelta(text) = &ev {
            if let Ok(mut tail) = tail.lock() {
                push_tail(&mut tail, text, MAX_TRANSCRIPT_BYTES);
            }
        }
    };
    let result = {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("agent-step-runner: cannot start tokio runtime: {error}");
                return Err(1);
            }
        };
        runtime.block_on(run_session(&mut session, prompt, on_event))
    };

    if let Some(error) = event_error.lock().unwrap().as_ref() {
        eprintln!("agent-step-runner: event persistence failed: {error}");
        return Err(1);
    }
    // 6. Artifacts: transcript (fallback to the last assistant message),
    // optional structured output, terminal session.json.
    let mut text = transcript.lock().map(|t| t.clone()).unwrap_or_default();
    if text.trim().is_empty() {
        if let Some(completed) = opencoder_session::handoff::last_assistant_text(&session.messages)
        {
            text = completed;
        }
    }
    let _ = std::fs::write(step_dir.join("transcript.txt"), &text);
    if let Some(value) = opencoder_dag_runtime::exec::agent::extract_output_json_from(&text) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
            let _ = std::fs::write(step_dir.join("output.json"), bytes);
        }
    }
    match &result {
        Ok(()) => write_session_json(
            &step_dir,
            &session_id,
            &agent_name,
            config.model_id(),
            "done",
            None,
        ),
        Err(error) => write_session_json(
            &step_dir,
            &session_id,
            &agent_name,
            config.model_id(),
            "error",
            Some(&format!("{error:#}")),
        ),
    }
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("agent-step-runner: session run failed: {error:#}");
            Err(1)
        }
    }
}

/// `session.json` body: the host's `{"session_id": ...}` pointer extended
/// with the runner's own liveness/status fields (additive keys).
fn write_session_json(
    step_dir: &std::path::Path,
    session_id: &str,
    agent: &str,
    model: &str,
    status: &str,
    error: Option<&str>,
) {
    let mut body = serde_json::json!({
        "session_id": session_id,
        "agent": agent,
        "model": model,
        "status": status,
    });
    if let Some(error) = error {
        body["error"] = serde_json::Value::String(error.to_string());
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&body) {
        let _ = std::fs::write(step_dir.join("session.json"), bytes);
    }
}

/// Append `delta`, then trim to the last `max` bytes on a char boundary
/// (same seam discipline as `exec::agent`'s bounded transcript tail).
fn push_tail(tail: &mut String, delta: &str, max: usize) {
    tail.push_str(delta);
    if tail.len() > max {
        let mut cut = tail.len() - max;
        while cut < tail.len() && !tail.is_char_boundary(cut) {
            cut += 1;
        }
        let kept = tail[cut..].to_string();
        *tail = kept;
    }
}
