use std::path::PathBuf;
use std::sync::Arc;

use std::time::Duration;
use tokio_util::sync::CancellationToken;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::{
    generate_title, resume_and_replay as resume_session, run_once, SessionState,
};
use opencoder_store::{SessionFilter, SessionPatch, Store};

use crate::display::{print_event, truncate};
use crate::Cli;

/// Apply a `--model` override (format `provider/model_id`) to the config.
/// Must be called before `resolve_endpoint` so the LLM client is built against
/// the chosen provider's credentials. Returns true when the config changed.
pub(crate) fn apply_model_override(config: &mut Config, model: &Option<String>) -> bool {
    if let Some(m) = model {
        if config.model != *m {
            config.model = m.clone();
            return true;
        }
    }
    false
}

/// Re-apply an explicit `--model` to a resumed session. `resume()` restores the
/// stored model into the session, so an explicit `--model` must win here. Returns
/// the new model string when the session was changed (caller persists it), else None.
pub(crate) fn reapply_resume_model(
    session: &mut SessionState,
    model: &Option<String>,
) -> Option<String> {
    let m = model.as_ref()?;
    if session.config.model == *m {
        return None;
    }
    session.config.model = m.clone();
    session.model = session.config.model_id().to_string();
    Some(m.clone())
}

/// Apply an `--agent` override (builtin name like plan/explore/build) to the
/// config. Sets `config.agent.default` so the fresh-session path resolves it.
/// Returns true when the config changed.
pub(crate) fn apply_agent_override(config: &mut Config, agent: &Option<String>) -> bool {
    if let Some(a) = agent {
        if config.agent.default != *a {
            config.agent.default = a.clone();
            return true;
        }
    }
    false
}

/// Re-apply an explicit `--agent` to a resumed session. `resume()` restores the
/// stored agent into the session, so an explicit `--agent` must win here. Returns
/// the new agent name when the session was changed (caller persists it), else None.
pub(crate) fn reapply_resume_agent(
    session: &mut SessionState,
    agent: &Option<String>,
) -> Result<Option<String>> {
    let name = match agent.as_ref() {
        Some(n) => n,
        None => return Ok(None),
    };
    if session.agent.name == *name {
        return Ok(None);
    }
    // `name` here is always an explicit --agent value (we returned early on
    // None), so an unknown name must error rather than silently resolve to
    // "act".
    let resolved = resolve_agent(name).ok_or_else(|| anyhow!("agent not found: {name}"))?;
    session.agent = resolved;
    Ok(Some(name.clone()))
}

pub async fn run_headless(cli: &Cli, prompt: String) -> Result<()> {
    // --fork copies a resumed session, so it is meaningless without a resume
    // target. Without this guard, `--fork` on its own silently creates a fresh
    // session (pick_resume_id returns Ok(None) and cli.fork is never read).
    if cli.fork && cli.session.is_none() && !cli.continue_ {
        anyhow::bail!("--fork requires --session <id> or --continue");
    }
    let workdir = resolve_workdir(cli)?;
    let mut config = Config::load(&workdir)?;
    apply_model_override(&mut config, &cli.model);
    apply_agent_override(&mut config, &cli.agent);
    let ep = config.resolve_endpoint()?;
    let client: Arc<dyn ChatStream> = Arc::new(ChatClient::new_with_read_timeout(
        &ep.base_url,
        &ep.api_key,
        &ep.headers,
        config.stream_idle_timeout(),
        config.network.proxy.as_deref(),
    )?);
    let store: Option<Arc<dyn Store>> = crate::session_cmd::open_store(&workdir)
        .await
        .ok()
        .map(|s| Arc::new(s) as Arc<dyn Store>);

    // Create the cancellation token up front so recovery (resume_and_replay) is
    // itself interruptible: a Ctrl-C during replay cancels the token, which
    // replay_child races against, instead of freezing until the child finishes.
    let cancel = CancellationToken::new();
    let mut session = if let Some(id) = pick_resume_id(cli, store.as_deref()).await? {
        let st = store
            .clone()
            .ok_or_else(|| anyhow!("store unavailable for resume"))?;
        let effective_id = if cli.fork {
            fork_session(st.as_ref(), &id).await?
        } else {
            id
        };
        resume_session(
            st,
            &effective_id,
            config.clone(),
            client.clone(),
            workdir.clone(),
            Some(cancel.clone()),
        )
        .await?
    } else {
        let agent_name = config.agent.default.as_str();
        // Only fall back to "act" when no agent name was configured at all.
        // An explicit but unknown name (e.g. a typo via --agent or config) must
        // error rather than silently resolve to "act".
        let agent = if agent_name.is_empty() {
            resolve_agent("act").ok_or_else(|| anyhow!("agent not found: act"))?
        } else {
            resolve_agent(agent_name)
                .ok_or_else(|| anyhow!("agent not found: {agent_name}"))?
        };
        let mut s = SessionState::new(
            opencoder_session::runner::new_id(),
            agent,
            config.clone(),
            client.clone(),
            workdir.clone(),
        );
        if let Some(st) = &store {
            s = s.with_store(st.clone());
        }
        s
    };

    // resume() restored the session's stored model; an explicit --model wins
    // over it and is re-persisted so subsequent resumes honor the new choice.
    if let Some(new_model) = reapply_resume_model(&mut session, &cli.model) {
        if let Some(st) = &store {
            let _ = st
                .update_session(
                    &session.id,
                    &SessionPatch {
                        model: Some(new_model),
                        updated_at: Some(opencoder_core::message::now_ms()),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    // Likewise, an explicit --agent wins over the resumed session's stored
    // agent and is re-persisted so subsequent resumes honor the new choice.
    if let Some(new_agent) = reapply_resume_agent(&mut session, &cli.agent)? {
        if let Some(st) = &store {
            let _ = st
                .update_session(
                    &session.id,
                    &SessionPatch {
                        agent: Some(new_agent),
                        updated_at: Some(opencoder_core::message::now_ms()),
                        ..Default::default()
                    },
                )
                .await;
        }
    }

    if session.store.is_none() {
        if let Some(st) = &store {
            session.store = Some(st.clone());
        }
    }

    print_resume_summary(&session).await;

    if let Some(pf) = &cli.prompt_file {
        let body = std::fs::read_to_string(pf)
            .map_err(|e| anyhow!("--prompt-file {}: {e}", pf.display()))?;
        session.agent.prompt = format!("{}\n\n{}", body.trim(), opencoder_core::tool_preamble());
    }

    // Extract and resolve $skill-name tokens from the prompt.
    let prompt = {
        let (clean, names) = opencoder_core::extract_skill_tokens(&prompt);
        if !names.is_empty() {
            let skills = opencoder_core::discover_skills();
            let mut resolved_bodies = Vec::new();
            let mut resolved_names = std::collections::HashSet::new();
            for name in &names {
                if let Some(sk) = skills.iter().find(|s| &s.name == name) {
                    resolved_bodies.push(sk.body.clone());
                    resolved_names.insert(sk.name.clone());
                }
            }
            if !resolved_bodies.is_empty() {
                let body = resolved_bodies.join("\n\n");
                session.set_skill(Some(body));
                session.set_active_skill_names(resolved_names);
            }
        }
        clean
    };

    print_prompt_header(&session, &prompt);

    // Read any --image attachments into base64 data URIs and attach them to
    // the first user message. An unreadable/missing file is a hard error
    // (fail loudly rather than silently dropping an attachment).
    let images = load_image_data_uris(&cli.image)?;

    // Attach a cancellation token so a hung tool/LLM call (which previously
    // had no headless escape hatch) can be interrupted: first Ctrl-C requests
    // a graceful stop at the next turn boundary / select! cancel arm; a second
    // Ctrl-C forces an immediate exit. Without this, `run_headless` would
    // block forever on a tool whose future never resolves.
    // Reuse the token created before resume so Ctrl-C also interrupts the run
    // loop (the session already holds it from resume; re-affirm for fresh ones).
    session.cancel = Some(cancel.clone());
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        // First Ctrl-C: ask the run loop to stop at the next await point it
        // can interrupt (turn boundary, LLM `rx.recv()`, or tool select!).
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\n\x1b[2m[interrupting\u{2026} press Ctrl-C again to force quit]\x1b[0m");
            cancel_for_signal.cancel();
        }
        // Second Ctrl-C: graceful stop did not satisfy the user; bail out.
        let _ = tokio::signal::ctrl_c().await;
        opencoder_session::tools::bg::cleanup_all();
        std::process::exit(130);
    });
    if images.is_empty() {
        opencoder_session::run(&mut session, prompt, |ev| print_event(&ev)).await?;
    } else {
        opencoder_session::run_with_images(&mut session, prompt, images, |ev| print_event(&ev))
            .await?;
    }

    // cheap background title generation (small model) after the first round.
    // This is a best-effort nicety: if the model is unreachable (e.g. the
    // endpoint hangs) or the user already pressed Ctrl-C, never block the exit
    // here. A 30 s cap is ample for a 64-token generation; on timeout/cancel
    // the session simply keeps its default (empty) title.
    if !cancel.is_cancelled() {
        let _ = tokio::time::timeout(Duration::from_secs(30), generate_title(&session)).await;
    }

    eprintln!("\n\x1b[2m[session {}]\x1b[0m", session.id);
    eprintln!("\x1b[2m{}\x1b[0m", resume_hint(&session.id));
    Ok(())
}

/// Resolve which session id to resume, honoring --session, then --continue.
///
/// When `--session <id>` is given, the ID is first tried as a session ID.
/// If no session matches, it is tried as a subagent `task_id` — if found,
/// the parent session is returned so the full parent context is resumed.
async fn pick_resume_id(cli: &Cli, store: Option<&dyn Store>) -> Result<Option<String>> {
    if let Some(id) = &cli.session {
        if let Some(s) = store {
            // Try as a session ID first.
            if s.get_session(id).await?.is_none() {
                // Not a session — try as a subagent task_id to find the
                // parent session that owns it.
                if let Some(task) = s.get_subagent_task(id).await? {
                    return Ok(Some(task.parent_session_id));
                }
            }
        }
        return Ok(Some(id.clone()));
    }
    if cli.continue_ {
        let s = store.ok_or_else(|| anyhow!("no store available for --continue"))?;
        let list = s
            .list_sessions(&SessionFilter {
                limit: 1,
                ..Default::default()
            })
            .await?;
        if list.is_empty() {
            // Without this, an empty list returns Ok(None) and run_headless
            // silently falls through to the fresh-session path, masking the
            // user's intent to resume.
            anyhow::bail!("no sessions to --continue in this workdir");
        }
        return Ok(list.into_iter().next().map(|i| i.id));
    }
    Ok(None)
}

/// Copy a session's meta and messages into a new session id, leaving the
/// original untouched. Returns the new id.
pub async fn fork_session(store: &dyn Store, parent_id: &str) -> Result<String> {
    let meta = store
        .get_session(parent_id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {parent_id}"))?;
    let messages = store.load_messages(parent_id).await?;
    let new_id = opencoder_session::runner::new_id();
    let now = opencoder_core::message::now_ms();
    let forked = opencoder_store::SessionMeta {
        id: new_id.clone(),
        title: meta.title.as_deref().map(|t| format!("{t} (fork)")),
        agent: meta.agent.clone(),
        model: meta.model.clone(),
        workdir_hash: meta.workdir_hash.clone(),
        created_at: now,
        updated_at: now,
        summary: meta.summary.clone(),
        summary_seq: meta.summary_seq,
        handoff_seq: meta.handoff_seq,
        handoff_plan: meta.handoff_plan.clone(),
        skill: meta.skill.clone(),
        task_type: None,
    };
    store.create_session(&forked).await?;
    if !messages.is_empty() {
        store.append_messages(&new_id, &messages).await?;
    }
    eprintln!("\n\x1b[2m[forked {parent_id} \u{2192} {new_id}]\x1b[0m");
    Ok(new_id)
}

#[allow(dead_code)]
pub async fn run_once_inline(
    agent_name: &str,
    config: Config,
    client: Arc<dyn ChatStream>,
    workdir: PathBuf,
    prompt: String,
) -> Result<SessionState> {
    run_once(agent_name, config, client, workdir, prompt, |_| {}).await
}

fn resolve_workdir(cli: &Cli) -> Result<PathBuf> {
    if let Some(w) = &cli.workdir {
        return Ok(w.clone());
    }
    std::env::current_dir().context("get current dir")
}

/// Format a one-line summary of a resumed session's subagent tasks (Gap D).
/// Returns `None` when there are no tasks (e.g. a fresh session) so the caller
/// can skip printing. Pure / synchronous so it is directly unit-testable.
pub(crate) fn format_resume_summary(
    tasks: &[opencoder_store::SubagentTaskRecord],
) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    use opencoder_store::SubagentStatus;
    let total = tasks.len();
    let done = tasks
        .iter()
        .filter(|t| t.status != SubagentStatus::Running)
        .count();
    let details: Vec<String> = tasks
        .iter()
        .map(|t| {
            let mark = match t.status {
                SubagentStatus::Completed => {
                    if t.ok == Some(false) {
                        "\u{2718}"
                    } else {
                        "\u{2714}"
                    }
                }
                SubagentStatus::Failed => "\u{2718}",
                SubagentStatus::Cancelled => "\u{2298}",
                // Unknown is a serde fallback; treat like still-in-flight.
                SubagentStatus::Running | SubagentStatus::Unknown => "\u{2026}",
            };
            format!("{mark} {}", truncate(&t.prompt, 40))
        })
        .collect();
    Some(format!(
        "\u{2937} resumed session: {done}/{total} subagents done \u{2014} {}",
        details.join(", ")
    ))
}

/// Print a one-line summary of the resumed session's subagent tasks so a
/// headless `opencode -s` user can see prior dispatches and their outcomes
/// (otherwise resume shows nothing about restored subagent context). Mirrors
/// the live `SubagentStart`/`SubagentEnd` glyph style. No-op when there are no
/// subagent tasks (e.g. a fresh session).
async fn print_resume_summary(session: &SessionState) {
    let store = match &session.store {
        Some(s) => s,
        None => return,
    };
    let tasks = match store.list_subagent_tasks(&session.id).await {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Some(line) = format_resume_summary(&tasks) {
        eprintln!("\x1b[34m{line}\x1b[0m");
    }
}

fn print_prompt_header(_session: &SessionState, prompt: &str) {
    eprintln!("\n\x1b[1muser\x1b[0m: {}\n", prompt.trim_end());
}

/// Copy-paste-ready command to resume a session by id.
fn resume_hint(id: &str) -> String {
    format!("resume with: opencoder -s {id}")
}

#[allow(dead_code)]
pub fn _duration() -> Duration {
    Duration::from_secs(0)
}

/// Read each `--image` file path into a `data:image/<fmt>;base64,<...>` URI
/// suitable for the `ContentBlock::Image` / OpenAI `image_url` field. Returns
/// an empty vec when no paths were given. A missing/unreadable file errors.
pub(crate) fn load_image_data_uris(paths: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let path = std::path::Path::new(p);
        let bytes =
            std::fs::read(path).with_context(|| format!("--image {p}: cannot read file"))?;
        let mime = mime_from_ext(path);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        out.push(format!("data:{mime};base64,{b64}"));
    }
    Ok(out)
}

/// Map a file extension to an image MIME type. Unknown extensions fall back to
/// `image/png`, the most widely supported default for vision endpoints.
fn mime_from_ext(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_hint_is_copyable_command() {
        assert_eq!(resume_hint("01ABC"), "resume with: opencoder -s 01ABC");
    }

    #[test]
    fn format_resume_summary_lists_subagents() {
        use opencoder_store::{SubagentStatus, SubagentTaskRecord};
        fn task(
            id: &str,
            agent: &str,
            prompt: &str,
            status: SubagentStatus,
            ok: Option<bool>,
        ) -> SubagentTaskRecord {
            SubagentTaskRecord {
                task_id: id.into(),
                parent_session_id: "p".into(),
                child_session_id: format!("c-{id}"),
                parent_message_id: None,
                agent: agent.into(),
                prompt: prompt.into(),
                result: None,
                status,
                ok,
                started_at: 0,
                completed_at: Some(1),
            }
        }
        // Empty -> None.
        assert!(format_resume_summary(&[]).is_none());

        let tasks = vec![
            task(
                "t1",
                "explore",
                "find all TODO comments",
                SubagentStatus::Completed,
                Some(true),
            ),
            task(
                "t2",
                "build",
                "fix the bug in module foo bar baz qux",
                SubagentStatus::Failed,
                Some(false),
            ),
        ];
        let s = format_resume_summary(&tasks).expect("non-empty -> Some");
        assert!(s.contains("2/2 subagents done"), "got: {s}");
        assert!(s.contains('\u{2714}'), "completed mark (✔) present: {s}");
        assert!(s.contains('\u{2718}'), "failed mark (✘) present: {s}");
        assert!(
            s.contains("find all TODO comments"),
            "explore prompt present: {s}"
        );
        assert!(
            s.contains("fix the bug in module foo bar baz qux"),
            "build prompt present: {s}"
        );

        // A Running task counts toward total but not done.
        let running = vec![task(
            "r",
            "explore",
            "still going",
            SubagentStatus::Running,
            None,
        )];
        let s = format_resume_summary(&running).expect("Some");
        assert!(
            s.contains("0/1 subagents done"),
            "running not counted as done: {s}"
        );
        assert!(s.contains('\u{2026}'), "running mark (…) present: {s}");
    }

    #[tokio::test]
    async fn pick_resume_id_resolves_task_id_to_parent_session() {
        use clap::Parser;
        use opencoder_store::{
            LibsqlStore, SessionMeta, Store, SubagentStatus, SubagentTaskRecord,
        };

        let store = LibsqlStore::open_memory().await.unwrap();

        // Create a parent session.
        let parent_id = "parent-sess";
        store
            .create_session(&SessionMeta {
                id: parent_id.into(),
                title: Some("parent".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
            })
            .await
            .unwrap();

        // Create a child session (required by FK constraint on subagent_tasks).
        store
            .create_session(&SessionMeta {
                id: "sub-sess-001".into(),
                title: None,
                agent: None,
                model: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
            })
            .await
            .unwrap();

        // Create a subagent task whose task_id should resolve to the parent.
        let task_id = "task-001";
        store
            .create_subagent_task(&SubagentTaskRecord {
                task_id: task_id.into(),
                parent_session_id: parent_id.into(),
                child_session_id: "sub-sess-001".into(),
                parent_message_id: Some("msg-42".into()),
                agent: "explore".into(),
                prompt: "find all TODO comments".into(),
                result: None,
                status: SubagentStatus::Running,
                ok: None,
                started_at: 1000,
                completed_at: None,
            })
            .await
            .unwrap();

        // `--session <task_id>` should resolve to the parent session id.
        let cli = Cli::parse_from(["opencoder", "--session", task_id]);
        let resolved = pick_resume_id(&cli, Some(&store as &dyn Store))
            .await
            .unwrap();
        assert_eq!(resolved.as_deref(), Some(parent_id));
    }

    #[tokio::test]
    async fn pick_resume_id_returns_real_session_as_is() {
        use clap::Parser;
        use opencoder_store::{LibsqlStore, SessionMeta, Store};

        let store = LibsqlStore::open_memory().await.unwrap();

        // Create a real session.
        let session_id = "real-sess";
        store
            .create_session(&SessionMeta {
                id: session_id.into(),
                title: None,
                agent: None,
                model: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
            })
            .await
            .unwrap();

        // `--session <session_id>` should be returned unchanged.
        let cli = Cli::parse_from(["opencoder", "--session", session_id]);
        let resolved = pick_resume_id(&cli, Some(&store as &dyn Store))
            .await
            .unwrap();
        assert_eq!(resolved.as_deref(), Some(session_id));
    }

    #[test]
    fn apply_model_override_sets_provider_model() {
        let mut cfg = Config::default();
        assert!(apply_model_override(
            &mut cfg,
            &Some("anthropic/claude-3".into())
        ));
        assert_eq!(cfg.model, "anthropic/claude-3");
        assert_eq!(cfg.provider_id(), "anthropic");
        assert_eq!(cfg.model_id(), "claude-3");
        // no override -> no change
        let mut cfg2 = Config::default();
        let before = cfg2.model.clone();
        assert!(!apply_model_override(&mut cfg2, &None));
        assert_eq!(cfg2.model, before);
    }

    #[test]
    fn reapply_resume_model_overrides_stored_model() {
        use opencoder_core::resolve_agent;
        use opencoder_llm::{ChatStream, MockChatClient};
        use opencoder_session::SessionState;
        use std::sync::Arc;
        // simulate a session resumed with stored model "openai/gpt-4o-mini"
        let cfg = Config {
            model: "openai/gpt-4o-mini".into(),
            ..Config::default()
        };
        let agent = resolve_agent("act").unwrap();
        let mut s = SessionState::new(
            "s1",
            agent,
            cfg,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        // explicit --model anthropic/claude-3 wins over stored model
        let changed = reapply_resume_model(&mut s, &Some("anthropic/claude-3".into()));
        assert_eq!(changed.as_deref(), Some("anthropic/claude-3"));
        assert_eq!(s.model, "claude-3");
        assert_eq!(s.config.provider_id(), "anthropic");
        // no override -> no change, returns None
        assert_eq!(reapply_resume_model(&mut s, &None), None);
    }

    #[test]
    fn apply_agent_override_sets_default() {
        let mut cfg = Config::default();
        assert_eq!(cfg.agent.default, "act");
        assert!(apply_agent_override(&mut cfg, &Some("plan".into())));
        assert_eq!(cfg.agent.default, "plan");
        // same value -> no change (returns false)
        assert!(!apply_agent_override(&mut cfg, &Some("plan".into())));
        // no override -> no change
        let mut cfg2 = Config::default();
        let before = cfg2.agent.default.clone();
        assert!(!apply_agent_override(&mut cfg2, &None));
        assert_eq!(cfg2.agent.default, before);
    }

    #[test]
    fn reapply_resume_agent_overrides_stored_agent() {
        use opencoder_core::resolve_agent;
        use opencoder_llm::{ChatStream, MockChatClient};
        use opencoder_session::SessionState;
        use std::sync::Arc;
        // simulate a session resumed with the default "act" agent
        let cfg = Config::default();
        let agent = resolve_agent("act").unwrap();
        let mut s = SessionState::new(
            "s1",
            agent,
            cfg,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        // explicit --agent plan wins over the resumed "act"
        let changed = reapply_resume_agent(&mut s, &Some("plan".into())).unwrap();
        assert_eq!(changed.as_deref(), Some("plan"));
        assert_eq!(s.agent.name, "plan");
        // same value -> no change, returns None
        assert_eq!(
            reapply_resume_agent(&mut s, &Some("plan".into())).unwrap(),
            None
        );
        // no override -> no change, returns None
        assert_eq!(reapply_resume_agent(&mut s, &None).unwrap(), None);
    }

    #[tokio::test]
    async fn fork_without_session_or_continue_errors() {
        use clap::Parser;
        // --fork with no --session/--continue must error rather than silently
        // creating a fresh session (the guard runs before any I/O).
        let cli = Cli::parse_from(["opencoder", "--fork"]);
        let err = run_headless(&cli, "hi".into()).await.unwrap_err();
        assert!(
            err.to_string().contains("--fork requires --session"),
            "expected --fork guard error, got: {err}"
        );
    }

    #[tokio::test]
    async fn continue_with_no_sessions_errors() {
        use clap::Parser;
        use opencoder_store::{LibsqlStore, Store};

        let store = LibsqlStore::open_memory().await.unwrap();
        // No sessions exist: --continue must error, not fall through to a
        // fresh-session creation.
        let cli = Cli::parse_from(["opencoder", "--continue"]);
        let err = pick_resume_id(&cli, Some(&store as &dyn Store))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no sessions to --continue"),
            "expected empty-continue error, got: {err}"
        );
    }

    #[test]
    fn reapply_resume_agent_rejects_unknown_name() {
        use opencoder_llm::{ChatStream, MockChatClient};
        use opencoder_session::SessionState;
        use std::sync::Arc;
        // A typo'd/explicit-but-unknown agent name must error rather than
        // silently resolving to "act".
        let cfg = Config::default();
        let agent = resolve_agent("act").unwrap();
        let mut s = SessionState::new(
            "s1",
            agent,
            cfg,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        let err = reapply_resume_agent(&mut s, &Some("nonexistent-agent".into()))
            .unwrap_err();
        assert!(
            err.to_string().contains("agent not found: nonexistent-agent"),
            "expected unknown-agent error, got: {err}"
        );
        // session agent unchanged by the failed reapply
        assert_eq!(s.agent.name, "act");
    }
}
