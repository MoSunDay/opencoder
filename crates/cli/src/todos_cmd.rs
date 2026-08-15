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
        let spec = opencoder_todos::parse_spec(&raw)?;
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
        TodosSub::List { json } => list(&store, *json).await,
        TodosSub::Interrupt { id } => {
            let state = opencoder_todos::interrupt(&store, id, "CLI interrupt requested").await?;
            println!(
                "{} suspended at generation {}",
                state.workflow_id, state.generation
            );
            Ok(())
        }
        TodosSub::Run { file, debug } => {
            let raw = tokio::fs::read_to_string(file)
                .await
                .with_context(|| format!("read todos file {}", file.display()))?;
            let spec = opencoder_todos::parse_spec(&raw)?;
            let workflow_id = format!("todos-{}", ulid::Ulid::new());
            println!("workflow_id={workflow_id}");
            let runtime = runtime(cli, &workdir, store, *debug)?;
            let state = runtime.run_new_with_id(spec, workflow_id).await?;
            print_terminal(&state)
        }
        TodosSub::Resume { id, debug } => {
            println!("workflow_id={id}");
            let runtime = runtime(cli, &workdir, store, *debug)?;
            let state = runtime.resume(id).await?;
            print_terminal(&state)
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
            let item = &state.todos[&todo.id];
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

async fn list(store: &Arc<dyn Store>, json: bool) -> Result<()> {
    let workflows = opencoder_todos::persistence::list(store).await?;
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

fn print_terminal(state: &opencoder_todos::WorkflowState) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(state)?);
    if state.status == opencoder_todos::types::WorkflowStatus::Completed {
        Ok(())
    } else {
        anyhow::bail!(
            "workflow ended as {}: {}",
            state.status.as_str(),
            state.terminal_reason.as_deref().unwrap_or("no reason")
        )
    }
}
