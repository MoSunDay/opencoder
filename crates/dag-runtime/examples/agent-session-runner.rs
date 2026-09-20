//! Container-side multi-turn agent session runner (`run_mode: agent`
//! custom agents under `sandbox: runc`).
//!
//! The host executes an agent session by launching THIS binary inside a
//! read-only OCI container (`ArgvStyle::Direct`, argv
//! `["/usr/bin/agent-session-runner"]`) once PER TURN; the whole session
//! turn -- LLM loop, tools, artifacts -- runs confined to the container with
//! the per-session rw dir, the pinned read-only agents pool
//! (`/workspace/agent`) and optionally the knowledge root exposed.
//! Continuation is file-based: the host stages `<step_dir>/messages.json`
//! from its durable store before each turn, and persists the delta from
//! the `messages.json` this runner writes after the turn.
//!
//! Contract (env, injected by the host executor):
//! - `OPENCODER_STEP_PROMPT`  -- container path of THIS turn's prompt file
//!   (REQUIRED; missing = contract violation, exit code 2);
//! - `OPENCODER_STEP_DIR`     -- the per-session rw dir (default
//!   `/workspace/context/step`);
//! - `OPENCODER_STEP_SESSION_ID` -- host session id (a fresh ULID when
//!   absent);
//! - `OPENCODER_STEP_AGENT`   -- executing agent name (default `act`);
//! - `OPENCODER_AGENTS_DIR`   -- container path of the pinned agents pool
//!   (default `/workspace/agent`); re-pinned process-wide BEFORE any agent
//!   resolution so `resolve_agent`, skill roots, tools paths and memory
//!   all read the read-only pool;
//! - `OPENAI_BASE_URL` / `OPENAI_API_KEY` / `OPENCODER_MODEL` -- the host's
//!   resolved LLM endpoint, applied through `Config`'s env overlay;
//! - `OPENCODER_HOW_APPEND`   -- optional how.md payload.
//!
//! Artifacts written into the step dir (host-visible through the bind):
//! - `events.ndjson` -- every `SessionEvent` of this turn, one
//!   `{"kind": ..., "payload": ...}` line per event in the SSE wire shape
//!   (`SessionEvent::sse_kind`/`sse_data`; the host reconstructs events
//!   with `SessionEvent::from_sse` and tails the file while the turn
//!   runs). Lines are written + flushed eagerly.
//! - `messages.json` -- the full `session.messages` after the turn (the
//!   host persists the delta; the next turn seeds from this file);
//! - `session.json` (`running` -> `done`/`error`), `transcript.txt`
//!   (bounded tail), and the optional `output.json` recovered from the
//!   final assistant text with the same extraction contract as the host
//!   path.
//!
//! Exit codes: 0 success, 1 run failure, 2 contract violation.

use std::io::Write as _;
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

/// Returns the process exit code (0/1/2) after running ONE turn.
fn run() -> Result<(), i32> {
    // 1. Pin the agents pool BEFORE anything resolves agents:
    //    `agents_dir()` in core reads `OPENCODER_AGENTS_DIR`, so with this
    //    set, `resolve_agent`, skill roots, tools paths and memory all hit
    //    the read-only mounted pool for the whole process. This happens
    //    before the tokio runtime exists, so the env mutation is
    //    single-threaded.
    let agents_dir = env_or("OPENCODER_AGENTS_DIR", "/workspace/agent");
    std::env::set_var("OPENCODER_AGENTS_DIR", &agents_dir);

    // 2. Turn contract: the prompt file must exist before anything else.
    let prompt_path = match std::env::var("OPENCODER_STEP_PROMPT") {
        Ok(path) if !path.is_empty() => PathBuf::from(path),
        _ => {
            eprintln!("agent-session-runner: OPENCODER_STEP_PROMPT is required (prompt file path)");
            return Err(2);
        }
    };
    let prompt = match std::fs::read_to_string(&prompt_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "agent-session-runner: cannot read prompt file {}: {error}",
                prompt_path.display()
            );
            return Err(2);
        }
    };
    let step_dir = PathBuf::from(env_or("OPENCODER_STEP_DIR", "/workspace/context/step"));
    if let Err(error) = std::fs::create_dir_all(&step_dir) {
        eprintln!(
            "agent-session-runner: cannot create step dir {}: {error}",
            step_dir.display()
        );
        return Err(2);
    }
    let session_id = env_or("OPENCODER_STEP_SESSION_ID", &ulid::Ulid::new().to_string());
    let agent_name = env_or("OPENCODER_STEP_AGENT", "act");

    // 3. Config: the container carries no config files, so `/workspace`
    //    discovery plus the host-injected env overlay defines the endpoint.
    let config = match opencoder_core::Config::load(std::path::Path::new("/workspace")) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "agent-session-runner: config load failed ({error}); falling back to defaults"
            );
            opencoder_core::Config::default()
        }
    };
    let endpoint = match config.resolve_endpoint() {
        Ok(endpoint) => endpoint,
        Err(error) => {
            eprintln!("agent-session-runner: cannot resolve LLM endpoint: {error}");
            return Err(1);
        }
    };
    let client: Arc<dyn ChatStream> = match ChatClient::from_config(&config, &endpoint) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            eprintln!("agent-session-runner: cannot build LLM client: {error}");
            return Err(1);
        }
    };
    let agent = match opencoder_core::resolve_agent(&agent_name) {
        Some(agent) => agent,
        None => {
            eprintln!("agent-session-runner: unknown agent `{agent_name}`");
            return Err(1);
        }
    };

    // 4. Session pinned to the step dir (writable mount). Multi-turn
    //    continuation: when the host staged `<step_dir>/messages.json`
    //    from its durable store, seed the session history from it so this
    //    turn continues the conversation instead of starting fresh.
    let mut session = SessionState::new(
        session_id.clone(),
        agent,
        config.clone(),
        client,
        step_dir.clone(),
    );
    if let Ok(bytes) = std::fs::read(step_dir.join("messages.json")) {
        match serde_json::from_slice::<Vec<opencoder_core::Message>>(&bytes) {
            Ok(history) => session.messages = history,
            Err(error) => {
                eprintln!("agent-session-runner: cannot parse messages.json: {error}");
                return Err(2);
            }
        }
    }
    if let Ok(how_append) = std::env::var("OPENCODER_HOW_APPEND") {
        if !how_append.is_empty() {
            session
                .env_passthrough
                .push(("OPENCODER_HOW_APPEND".into(), how_append));
        }
    }
    if std::env::var("OPENCODER_KNOWLEDGE_DIR").is_ok_and(|v| !v.is_empty()) {
        // A read-only knowledge checkout must never refresh a git index.
        session
            .env_passthrough
            .push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));
    }

    // 5. Publish the live pointer immediately (the host wrote a minimal
    //    `session.json`; this richer body keeps the same `session_id` key).
    write_session_json(
        &step_dir,
        &session_id,
        &agent_name,
        config.model_id(),
        "running",
        None,
    );

    // 6. Event sink: the bounded transcript tail plus one ndjson line per
    //    event, appended eagerly (the host tails the file live).
    let events_path = step_dir.join("events.ndjson");
    let events_file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_path)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!(
                "agent-session-runner: cannot open {}: {error}",
                events_path.display()
            );
            return Err(1);
        }
    };
    let transcript = Arc::new(Mutex::new(String::new()));
    let events = Arc::new(Mutex::new(events_file));
    let tail = Arc::clone(&transcript);
    let sink = Arc::clone(&events);
    let on_event = move |ev: SessionEvent| {
        if let SessionEvent::TextDelta(text) = &ev {
            if let Ok(mut tail) = tail.lock() {
                push_tail(&mut tail, text, MAX_TRANSCRIPT_BYTES);
            }
        }
        append_event(&sink, &ev);
    };

    // 7. Run exactly one turn of the (possibly continued) session.
    let result = {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("agent-session-runner: cannot start tokio runtime: {error}");
                return Err(1);
            }
        };
        runtime.block_on(run_session(&mut session, prompt, on_event))
    };

    // 8. Artifacts: the full message history (the host persists the
    //    delta), the transcript tail (fallback to the last assistant
    //    message), optional structured output, terminal session.json.
    if let Ok(bytes) = serde_json::to_vec_pretty(&session.messages) {
        let _ = std::fs::write(step_dir.join("messages.json"), bytes);
    }
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
            eprintln!("agent-session-runner: session run failed: {error:#}");
            Err(1)
        }
    }
}

/// Append one event as an ndjson line to `events.ndjson`: the SSE wire
/// shape `{"kind": ..., "payload": ...}` (`SessionEvent::sse_kind` +
/// `sse_data`), which the host reconstructs through
/// `SessionEvent::from_sse`. Written and flushed eagerly -- the host tails
/// the file while the turn runs. Best-effort: I/O failures drop the line
/// (the turn's terminal artifacts remain authoritative).
fn append_event(sink: &Mutex<std::fs::File>, ev: &SessionEvent) {
    let line = serde_json::json!({
        "kind": ev.sse_kind(),
        "payload": ev.sse_data(),
    });
    let Ok(mut file) = sink.lock() else {
        return;
    };
    let mut bytes = match serde_json::to_vec(&line) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    bytes.push(b'\n');
    let _ = file.write_all(&bytes);
    let _ = file.flush();
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
