use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use opencoder_core::Config;
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_store::Store;
use tokio_util::sync::CancellationToken;

use crate::{Cli, TodosSub};

pub async fn dispatch(cli: &Cli, sub: &TodosSub) -> Result<()> {
    if let TodosSub::Validate { file } = sub {
        let raw = tokio::fs::read_to_string(file)
            .await
            .with_context(|| format!("read todos file {}", file.display()))?;
        let spec = opencoder_todos::parse_spec(&raw)
            .with_context(|| format!("parse todos spec {}", file.display()))?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "valid": true,
                "workflow_id": spec.id,
                "todo_count": spec.todos.len()
            }))?
        );
        return Ok(());
    }
    let workdir = cli
        .workdir
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    let store: Arc<dyn Store> = Arc::new(crate::session_cmd::open_store(&workdir).await?);
    match sub {
        TodosSub::Validate { .. } => unreachable!("validated before Store initialization"),
        TodosSub::Show { id, json } => show(&store, id, *json).await,
        TodosSub::Events { id, after, json } => events(&store, id, *after, *json).await,
        TodosSub::List { json, limit } => list(&store, *json, *limit).await,
        TodosSub::Interrupt { id } => {
            let state = opencoder_todos::interrupt(&store, id, "CLI interrupt requested").await?;
            let (spec, _) = opencoder_todos::persistence::load(&store, id)
                .await?
                .with_context(|| format!("todo workflow not found after interrupt: {id}"))?;
            let debug_root = opencoder_core::data_dir_for(&workdir).join("todos");
            opencoder_todos::persistence::refresh_debug_dump_if_present(
                &store,
                &spec,
                &state,
                &debug_root,
            )
            .await?;
            println!(
                "{} suspended at generation {}",
                state.workflow_id, state.generation
            );
            Ok(())
        }
        TodosSub::Run { file, debug, json } => {
            let raw = tokio::fs::read_to_string(file)
                .await
                .with_context(|| format!("read todos file {}", file.display()))?;
            let spec = opencoder_todos::parse_spec(&raw)
                .with_context(|| format!("parse todos spec {}", file.display()))?;
            let workflow_id = format!("todos-{}", ulid::Ulid::new());
            eprintln!("workflow_id={workflow_id}");
            let runtime = runtime(cli, &workdir, store.clone(), *debug)?;
            let state = track_progress(
                &store,
                &workflow_id,
                0,
                runtime.run_new_with_id(spec, workflow_id.clone()),
            )
            .await?;
            finish_state(&state, *json)
        }
        TodosSub::Resume { id, debug, json } => {
            eprintln!("workflow_id={id}");
            let runtime = runtime(cli, &workdir, store.clone(), *debug)?;
            let state = track_progress(
                &store,
                id,
                latest_event_seq(&store, id).await,
                runtime.resume(id),
            )
            .await?;
            finish_state(&state, *json)
        }
    }
}

fn runtime(
    cli: &Cli,
    workdir: &Path,
    store: Arc<dyn Store>,
    debug: bool,
) -> Result<opencoder_todos::Runtime> {
    let mut config = Config::load(workdir)?;
    crate::run::apply_model_override(&mut config, &cli.model);
    let endpoint = config.resolve_endpoint()?;
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new_with_read_timeout(
        &endpoint.base_url,
        &endpoint.api_key,
        &endpoint.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )?);
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal.cancel();
        }
    });
    Ok(opencoder_todos::Runtime {
        store,
        client,
        config,
        workdir: workdir.to_path_buf(),
        debug_root: debug.then(|| opencoder_core::data_dir_for(workdir).join("todos")),
        cancel,
    })
}

async fn show(store: &Arc<dyn Store>, id: &str, json: bool) -> Result<()> {
    let (spec, state) = opencoder_todos::persistence::load(store, id)
        .await?
        .with_context(|| format!("todo workflow not found: {id}"))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"spec":spec,"state":state}))?
        );
    } else {
        println!(
            "{} [{}] generation={}",
            spec.name,
            state.status.as_str(),
            state.generation
        );
        for todo in spec.todos {
            let item = state
                .todos
                .get(&todo.id)
                .with_context(|| format!("state missing TODO {}", todo.id))?;
            println!(
                "  {:<24} {:<16} attempt={} session={}",
                todo.id,
                item.status.as_str(),
                item.attempt,
                item.active_session_id.as_deref().unwrap_or("-")
            );
        }
    }
    Ok(())
}

async fn events(store: &Arc<dyn Store>, id: &str, after: i64, json: bool) -> Result<()> {
    opencoder_todos::persistence::load(store, id)
        .await?
        .with_context(|| format!("todo workflow not found: {id}"))?;
    let events = store.todo_events_after(id, after).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&events)?);
    } else {
        for event in events {
            println!(
                "{} {} {}",
                event.seq.unwrap_or_default(),
                event.kind,
                event.payload
            );
        }
    }
    Ok(())
}

async fn list(store: &Arc<dyn Store>, json: bool, limit: u32) -> Result<()> {
    let workflows = opencoder_todos::persistence::list(store, limit).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&workflows)?);
    } else {
        for workflow in workflows {
            println!(
                "{} {:<12} generation={}",
                workflow.id, workflow.status, workflow.generation
            );
        }
    }
    Ok(())
}

/// Pure render of the final workflow state document: pretty multi-line JSON
/// by default, single-line compact JSON when `json` is set. stdout carries
/// ONLY this document (no `workflow_id=` prefix, no trailing commentary), so
/// both shapes must stay free of any prefix/annotation text.
///
/// Serialization of `WorkflowState` is total in practice (plain strings,
/// numbers, and string-keyed maps), so the impossible-serializer-failure case
/// panics rather than silently emitting a truncated document.
pub fn render_final_state(state: &opencoder_todos::WorkflowState, json: bool) -> String {
    if json {
        serde_json::to_string(state).expect("serialize WorkflowState compact")
    } else {
        serde_json::to_string_pretty(state).expect("serialize WorkflowState pretty")
    }
}

/// Terminal outcome of a finished todos workflow, mapped one-to-one onto the
/// CLI's process-exit contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodosOutcome {
    /// All todos passed → exit 0.
    Completed,
    /// Locally requested Ctrl-C suspension → exit 130 (resumable).
    Interrupted,
    /// Any other terminal state → error exit 1, carrying the status name.
    Ended(&'static str),
}

/// Pure classification of a terminal [`WorkflowState`](opencoder_todos::WorkflowState):
/// `Completed` when the workflow finished; `Interrupted` only when it was
/// Suspended *by a local Ctrl-C* (`terminal_reason == "local interrupt
/// requested"`); `Ended(status)` otherwise — including `Suspended` under any
/// other reason (doom-loop guard, acceptance stall, …), which must NOT be
/// treated as a user interrupt.
pub fn todos_terminal_outcome(state: &opencoder_todos::WorkflowState) -> TodosOutcome {
    use opencoder_todos::types::WorkflowStatus;
    match state.status {
        WorkflowStatus::Completed => TodosOutcome::Completed,
        WorkflowStatus::Suspended
            if state.terminal_reason.as_deref() == Some("local interrupt requested") =>
        {
            TodosOutcome::Interrupted
        }
        _ => TodosOutcome::Ended(state.status.as_str()),
    }
}

/// Print the final workflow state; stdout contains ONLY this JSON document.
fn print_final_state(state: &opencoder_todos::WorkflowState, json: bool) -> Result<()> {
    println!("{}", render_final_state(state, json));
    Ok(())
}

/// Print the final state, then map the outcome to a process exit code:
/// completed → 0, local Ctrl-C suspension → 130, anything else → error (1).
fn finish_state(state: &opencoder_todos::WorkflowState, json: bool) -> Result<()> {
    print_final_state(state, json)?;
    match todos_terminal_outcome(state) {
        TodosOutcome::Completed => Ok(()),
        TodosOutcome::Interrupted => {
            eprintln!(
                "[todos] interrupted (Ctrl-C), workflow suspended — resume with: opencoder todos resume {}",
                state.workflow_id
            );
            std::process::exit(130);
        }
        TodosOutcome::Ended(status) => anyhow::bail!(
            "workflow ended as {}: {}",
            status,
            state.terminal_reason.as_deref().unwrap_or("no reason")
        ),
    }
}

/// Run the workflow future while tailing its transition events to stderr.
/// After the future resolves the tailer is aborted and remaining events are
/// drained once, so no late transition is silently dropped.
async fn track_progress<F>(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    start_seq: i64,
    run: F,
) -> Result<opencoder_todos::WorkflowState>
where
    F: std::future::Future<Output = Result<opencoder_todos::WorkflowState>>,
{
    let cursor = Arc::new(std::sync::atomic::AtomicI64::new(start_seq));
    let tailer = spawn_progress_tailer(store, workflow_id, &cursor);
    let result = run.await;
    tailer.abort();
    drain_progress(store, workflow_id, &cursor).await;
    result
}

fn spawn_progress_tailer(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    cursor: &Arc<std::sync::atomic::AtomicI64>,
) -> tokio::task::JoinHandle<()> {
    let store = store.clone();
    let workflow_id = workflow_id.to_string();
    let cursor = cursor.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            drain_progress(&store, &workflow_id, &cursor).await;
        }
    })
}

/// Print one human line per new event to stderr; never touches stdout.
async fn drain_progress(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    cursor: &Arc<std::sync::atomic::AtomicI64>,
) {
    let after = cursor.load(std::sync::atomic::Ordering::Relaxed);
    let Ok(events) = store.todo_events_after(workflow_id, after).await else {
        return;
    };
    for event in events {
        if let Some(seq) = event.seq {
            cursor.fetch_max(seq, std::sync::atomic::Ordering::Relaxed);
        }
        eprintln!("{}", describe_event(&event));
    }
}

/// Highest persisted event seq for the workflow (0 when none yet), so a
/// resumed run only reports events produced by this invocation.
async fn latest_event_seq(store: &Arc<dyn Store>, workflow_id: &str) -> i64 {
    store
        .todo_events_after(workflow_id, 0)
        .await
        .ok()
        .and_then(|events| events.into_iter().filter_map(|event| event.seq).max())
        .unwrap_or(0)
}

fn describe_event(event: &opencoder_store::TodoEventRecord) -> String {
    let payload = &event.payload;
    let field = |key: &str| payload[key].as_str().unwrap_or_default();
    match event.kind.as_str() {
        "todos_dispatched" => format!(
            "[todos] dispatch: {}",
            payload["todos"]
                .as_array()
                .map(|todos| todos
                    .iter()
                    .filter_map(|todo| todo["todo_id"].as_str())
                    .collect::<Vec<_>>()
                    .join(","))
                .unwrap_or_default()
        ),
        "todo_candidate_ready" => format!(
            "[todos] candidate ready: {} (gate ok={})",
            field("todo_id"),
            payload["gate"]["ok"]
        ),
        "todo_acceptance_started" => format!("[todos] accepting: {}", field("todo_id")),
        "todo_accepted" => format!("[todos] accepted: {}", field("todo_id")),
        "todo_revision_requested" => {
            format!("[todos] revise: {} — {}", field("todo_id"), field("reason"))
        }
        "todo_execution_failed" => {
            format!(
                "[todos] todo failed: {} — {}",
                field("todo_id"),
                field("reason")
            )
        }
        "todo_failed" => format!("[todos] todo failed (parent): {}", field("todo_id")),
        "workflow_rewound" => format!(
            "[todos] rewind to milestone: {}",
            field("milestone_todo_id")
        ),
        "workflow_completed"
        | "workflow_failed"
        | "workflow_suspended"
        | "workflow_interrupted"
        | "runtime_error"
        | "milestone_marked"
        | "workflow_resumed" => format!("[todos] {}: {}", event.kind, short_payload(payload)),
        _ => format!("[todos] {} {}", event.kind, payload),
    }
}

fn short_payload(payload: &serde_json::Value) -> String {
    payload["reason"]
        .as_str()
        .or_else(|| payload["error"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| payload.to_string())
}
