//! TUI bootstrap: config load, session resume/create, terminal setup.
//! Extracted from `app.rs` to keep that file under the 800-line iteration cap.

use std::sync::Arc;

use anyhow::{Context, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::ChatStream;
use opencoder_session::SessionState;
use opencoder_store::Store;
use ratatui::backend::CrosstermBackend;
use tokio_util::sync::CancellationToken;

use crate::app_helpers::{open_store, persist_session_model, reapply_session_model, resume_hint};
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
    Config::ensure_global_config().context("create ~/.opencoder/config.json")?;
    let mut config = Config::load(&workdir)?;
    crate::theme::set_theme(crate::theme::ThemeKind::from_label(&config.theme));
    if let Some(m) = &opts.model {
        config.model = m.clone();
    }
    let (config, concrete_client, active_terminal) =
        match crate::onboarding::build_ready_client(&config) {
            Ok(client) => (config, client, None),
            Err(startup_error) => {
                let mut terminal = ActiveTerminal::enter()?;
                match crate::onboarding::run(
                    &mut terminal.terminal,
                    &workdir,
                    opts.model.as_deref(),
                    config,
                    startup_error,
                )
                .await?
                {
                    crate::onboarding::OnboardingOutcome::Ready { config, client } => {
                        (*config, client, Some(terminal))
                    }
                    crate::onboarding::OnboardingOutcome::Exit => return Ok(()),
                }
            }
        };
    crate::theme::set_theme(crate::theme::ThemeKind::from_label(&config.theme));
    let client: Arc<dyn ChatStream> = Arc::new(concrete_client);

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
    let mut active_terminal = match active_terminal {
        Some(terminal) => terminal,
        None => ActiveTerminal::enter()?,
    };

    let result = super::run_app(
        &mut active_terminal.terminal,
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

    // Restore the tmux status bar and terminal before printing the hint.
    drop(active_terminal);
    let final_id = result?;
    eprintln!("\n\x1b[2m{}\x1b[0m", resume_hint(&final_id));
    Ok(())
}

/// A fully-entered terminal whose Drop restores both terminal state and the
/// tmux status bar. Keeping it as one value lets onboarding hand the same live
/// screen to the normal chat loop without a leave/re-enter flicker.
struct ActiveTerminal {
    terminal: Term,
    guard: Option<TerminalGuard>,
    tmux_bar_prev: Option<bool>,
}

impl ActiveTerminal {
    fn enter() -> Result<Self> {
        let tmux_bar_prev = crate::tmux_bar::hide();
        let guard = match TerminalGuard::enter() {
            Ok(guard) => guard,
            Err(error) => {
                crate::tmux_bar::restore(tmux_bar_prev);
                return Err(error);
            }
        };
        let backend = CrosstermBackend::new(std::io::stdout());
        let mut terminal = match Term::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                drop(guard);
                crate::tmux_bar::restore(tmux_bar_prev);
                return Err(error.into());
            }
        };
        // tmux retains its alternate-screen grid; force a clean first frame.
        if let Err(error) = terminal.clear() {
            drop(guard);
            crate::tmux_bar::restore(tmux_bar_prev);
            return Err(error.into());
        }
        Ok(Self {
            terminal,
            guard: Some(guard),
            tmux_bar_prev,
        })
    }
}

impl Drop for ActiveTerminal {
    fn drop(&mut self) {
        drop(self.guard.take());
        crate::tmux_bar::restore(self.tmux_bar_prev);
    }
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

#[cfg(test)]
mod tests {
    use super::finish;

    /// Normal exit path: `finish` disarms the supervisor flag and drops the
    /// command sender so the cooperative worker drains its channel and exits.
    #[tokio::test]
    async fn finish_disarms_supervisor_and_closes_channel_on_prompt_exit() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::mpsc;

        let supervisor_active = AtomicBool::new(true);
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<crate::worker::UiCmd>(4);
        // Cooperative worker: loops until the sender half is dropped.
        let worker = tokio::spawn(async move { while cmd_rx.recv().await.is_some() {} });

        finish(&supervisor_active, cmd_tx, worker).await;

        assert!(!supervisor_active.load(Ordering::Relaxed));
    }

    /// Stalled exit path: if the worker ignores cancellation, `finish` aborts
    /// it within its bounded 5 s timeout instead of hanging forever.
    #[tokio::test]
    async fn finish_aborts_stalled_worker_within_bound() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};
        use tokio::sync::mpsc;

        let supervisor_active = AtomicBool::new(true);
        let (cmd_tx, _cmd_rx) = mpsc::channel::<crate::worker::UiCmd>(4);
        // A worker that never completes on its own.
        let worker = tokio::spawn(async {
            std::future::pending::<()>().await;
        });

        let start = Instant::now();
        finish(&supervisor_active, cmd_tx, worker).await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(8),
            "finish should abort stalled worker within bound, took {:?}",
            elapsed
        );
        assert!(!supervisor_active.load(Ordering::Relaxed));
    }
}
