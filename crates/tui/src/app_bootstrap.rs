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
    crate::theme::set_theme(crate::theme::ThemeKind::from_label(&config.theme));
    if let Some(m) = &opts.model {
        config.model = m.clone();
    }
    let ep = startup_endpoint(&config)?;
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )?);

    let store: Arc<dyn Store> = open_store(&workdir).await?;
    // Mirror ts-owned sessions into the central ts registry (`<data_root>/ts.db`)
    // when one exists; a pure tui/run with no ts usage is unaffected.
    let store: Arc<dyn Store> = crate::ts_mirror::maybe_wrap(store, &workdir).await;

    // Resume an existing session if --session was given, otherwise start fresh.
    let replay_cancel = CancellationToken::new();
    let mut session = if let Some(id) = &opts.session {
        let existing = store.get_session(id).await?;
        // If not found as a session, try as a subagent task_id to resolve
        // the parent session.
        let task = if existing.is_none() {
            store.get_subagent_task(id).await?
        } else {
            None
        };
        if existing.is_none() && task.is_none() {
            // Unknown id — this is the tmux launch path where `ts_start`
            // allocated an id but deliberately did NOT seed a session row.
            // Create a fresh session that persists lazily on first record.
            let agent_name = config.agent.default.clone();
            let agent = resolve_agent(&agent_name)
                .or_else(|| resolve_agent("act"))
                .context("agent")?;
            SessionState::new(
                id.clone(),
                agent,
                config.clone(),
                client.clone(),
                workdir.clone(),
            )
            .with_store(store.clone())
            .ts_origin()
        } else {
            let effective_id = task
                .map(|t| t.parent_session_id)
                .unwrap_or_else(|| id.clone());
            opencoder_session::resume::resume_and_replay(
                store.clone(),
                &effective_id,
                config.clone(),
                client.clone(),
                workdir.clone(),
                Some(replay_cancel.clone()),
            )
            .await?
        }
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
    let context_limit = session.config.context_limit();
    let model_label = session.config.model.clone();

    // Terminal enter/restore is RAII: `TerminalGuard`'s Drop — and the panic
    // hook it installs — restore raw/alt-screen/mouse/kitty state on ANY exit
    // path (normal return, `?` error, or a panic that unwinds). This removes
    // the old "cleanup only ran on the happy path" trap that bricked the
    // terminal on any panic, leaving the user with a frozen last frame, no
    // echo, and ineffective Ctrl+C/D.
    let tmux_bar_prev = crate::tmux_bar::hide();
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Term::new(backend)?;
    // Entering the alt screen does NOT clear it: tmux keeps one persistent
    // alt-screen grid per pane, so a previous run's last frame (and any
    // status-bar hide / pane-resize edge rows) would show through wherever the
    // first draw's diff emits no bytes (empty-vs-empty cells are never
    // rewritten). A real `Terminal::clear()` sends ESC[2J and resets the diff
    // baseline so the first frame is a full repaint. Mirrors the
    // `resume_screen` contract in terminal.rs, which likewise expects a clear
    // after re-entering the alt screen.
    terminal.clear()?;

    let result = super::run_app(
        &mut terminal,
        session,
        store,
        session_id,
        compaction_threshold,
        context_limit,
        model_label,
        workdir,
        config,
        client,
    )
    .await;

    // Restore the tmux status bar and the real terminal on every exit path
    // (normal return or `?` error) before printing the resume hint.
    crate::tmux_bar::restore(tmux_bar_prev);
    drop(_guard);
    let final_id = result?;
    eprintln!("\n\x1b[2m{}\x1b[0m", resume_hint(&final_id));
    Ok(())
}

/// Disarm the liveness supervisor and bound the worker shutdown wait.
///
/// Called after the main event loop exits. The `cmd_tx` drop signals the
/// worker to stop; the 5-second timeout prevents a frozen terminal if a
/// tool or subagent ignores the cancellation.
pub(super) async fn finish(
    supervisor_active: &std::sync::atomic::AtomicBool,
    cmd_tx: tokio::sync::mpsc::Sender<crate::worker::UiCmd>,
    worker: tokio::task::JoinHandle<()>,
) {
    use std::sync::atomic::Ordering;
    supervisor_active.store(false, Ordering::Relaxed);
    drop(cmd_tx);
    let mut worker = worker;
    if tokio::time::timeout(std::time::Duration::from_secs(5), &mut worker)
        .await
        .is_err()
    {
        worker.abort();
    }
}
