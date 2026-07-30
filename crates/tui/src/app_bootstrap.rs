//! TUI bootstrap: config load, session resume/create, terminal setup.
//! Extracted from `app.rs` to keep that file under the 800-line iteration cap.

use std::sync::Arc;

use anyhow::{Context, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::SessionState;
use opencoder_store::Store;
use ratatui::backend::CrosstermBackend;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{
    open_store, persist_session_model, reapply_session_model, resume_hint, startup_endpoint,
};
use crate::render::Term;
use crate::terminal::TerminalGuard;
use crate::TuiOpts;

/// Entry point: load config, resume or create a session, enter the terminal,
/// then drive the event loop via `super::run_app`.
pub(super) async fn run(opts: &TuiOpts) -> Result<()> {
    let workdir = opts
        .workdir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let mut config = Config::load(&workdir)?;
    if let Some(m) = &opts.model {
        config.model = m.clone();
    }
    let ep = startup_endpoint(&config)?;
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.network.proxy.as_deref(),
    )?);

    let store: Arc<dyn Store> = open_store(&workdir).await?;

    // Resume an existing session if --session was given, otherwise start fresh.
    let replay_cancel = CancellationToken::new();
    let mut session = if let Some(id) = &opts.session {
        // Try as a session ID first; if not found, try as a subagent
        // task_id to resolve the parent session.
        let effective_id = if store.get_session(id).await?.is_none() {
            if let Some(task) = store.get_subagent_task(id).await? {
                task.parent_session_id
            } else {
                id.clone()
            }
        } else {
            id.clone()
        };
        opencoder_session::resume::resume_and_replay(
            store.clone(),
            &effective_id,
            config.clone(),
            client.clone(),
            workdir.clone(),
            Some(replay_cancel.clone()),
        )
        .await?
    } else {
        let agent_name = config.agent.default.clone();
        let agent = resolve_agent(&agent_name)
            .or_else(|| resolve_agent("act"))
            .context("agent")?;
        SessionState::new(
            opencoder_session::runner::new_id(),
            agent,
            config.clone(),
            client.clone(),
            workdir.clone(),
        )
        .with_store(store.clone())
    };

    // Explicit --model wins over a resumed session's stored model and is
    // re-persisted so later resumes honor it (headless run-path parity).
    if let Some(m) = reapply_session_model(&mut session, &opts.model) {
        persist_session_model(store.as_ref(), &session.id, m).await;
    }

    let session_id = session.id.clone();
    let compaction_threshold = session.config.compaction.context_threshold;
    let model_label = session.config.model.clone();

    // Terminal enter/restore is RAII: `TerminalGuard`'s Drop — and the panic
    // hook it installs — restore raw/alt-screen/mouse/kitty state on ANY exit
    // path (normal return, `?` error, or a panic that unwinds). This removes
    // the old "cleanup only ran on the happy path" trap that bricked the
    // terminal on any panic, leaving the user with a frozen last frame, no
    // echo, and ineffective Ctrl+C/D.
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Term::new(backend)?;

    let final_id = super::run_app(
        &mut terminal,
        session,
        store,
        session_id,
        compaction_threshold,
        model_label,
        workdir,
        config,
        client,
    )
    .await?;

    // Restore the real terminal *before* printing so the hint lands on the
    // actual screen instead of being swallowed by the alt-screen buffer.
    drop(_guard);
    eprintln!("\n\x1b[2m{}\x1b[0m", resume_hint(&final_id));
    Ok(())
}
