use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::time::Duration;
use tokio_util::sync::CancellationToken;

use anyhow::{anyhow, Context, Result};
use opencoder_core::{effective_default_agent, resolve_agent, AgentKind, Config};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::{generate_title, resume_and_replay as resume_session, SessionState};
use opencoder_store::{SessionFilter, SessionPatch, Store};

use crate::display::{print_event, truncate};
use crate::Cli;

pub(crate) use crate::run_image::load_image_data_uris;

pub(crate) use crate::model_override::{apply_model_override, reapply_resume_model};

pub(crate) use crate::agent_override::{apply_agent_override, reapply_resume_agent};

/// Legacy spelling compat for the sandbox-mode interlude: `/sandbox` was the
/// read-only mode switch while the dual mode was replaced; the canonical
/// spelling is `/plan` again. Rewrite a leading `/sandbox` control token
/// (bare or compound) so scripted inputs and muscle memory keep landing on
/// the plan agent. Anything that is not a leading control token passes
/// through unchanged (mid-sentence mentions, `/sandboxed`, plain prose).
/// Pure so it is directly unit-testable.
pub fn rewrite_legacy_sandbox_prefix(prompt: &str) -> String {
    let trimmed = prompt.trim();
    let rest = match trimmed.strip_prefix("/sandbox") {
        // `/sandbox` alone or `/sandbox ...` — never `/sandboxed` or "the
        // /sandbox doc".
        Some(r) if r.is_empty() || r.starts_with(char::is_whitespace) => r.trim(),
        _ => return prompt.to_string(),
    };
    if rest.is_empty() {
        "/plan".to_string()
    } else {
        format!("/plan {rest}")
    }
}

pub async fn run_headless(cli: &Cli, prompt: String) -> Result<()> {
    // Legacy compat: the sandbox-mode interlude spelled the read-only
    // switch `/sandbox`. The canonical spelling is `/plan` again, so an
    // unrewritten submission would reach the model as plain text. Rewrite the
    // control prefix before any resume/agent bookkeeping so every downstream
    // consumer (skill stripping, compound forwarding, the runner) sees `/plan`.
    let prompt = rewrite_legacy_sandbox_prefix(&prompt);
    // --fork copies a resumed session, so it is meaningless without a resume
    // target. Without this guard, `--fork` on its own silently creates a fresh
    // session (pick_resume_id returns Ok(None) and cli.fork is never read).
    if cli.fork && cli.session.is_none() && !cli.continue_ {
        anyhow::bail!("--fork requires --session <id> or --continue");
    }
    let workdir = resolve_workdir(cli)?;
    let mut config = Config::load(&workdir)?;
    // Malformed --model must fail here, before resolve_endpoint's api-key
    // error, so E20d's "names the malformed model" check holds.
    apply_model_override(&mut config, &cli.model).map_err(anyhow::Error::msg)?;
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
            let fid = opencoder_session::fork::fork_session(st.as_ref(), &id).await?;
            eprintln!("\n\x1b[2m[forked {id} \u{2192} {fid}]\x1b[0m");
            fid
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
        // Priority: explicit --agent (folded into the config above by
        // apply_agent_override) > active file-agent marker > cfg.agent.default
        // > "act". An explicit but unknown name (e.g. a typo via --agent or
        // config) must error rather than silently resolve to "act".
        let agent_name = effective_default_agent(cli.agent.as_deref(), &config);
        let agent =
            resolve_agent(&agent_name).ok_or_else(|| anyhow!("agent not found: {agent_name}"))?;
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
    if let Some(new_model) =
        reapply_resume_model(&mut session, &cli.model).map_err(anyhow::Error::msg)?
    {
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
        apply_prompt_file(&mut session, pf)?;
    }

    // Extract and resolve $skill-name tokens from the prompt.
    let prompt = {
        let raw = prompt.clone();
        let (clean, names) = opencoder_core::extract_skill_tokens(&prompt);
        let stripped = if names.is_empty() {
            clean
        } else {
            let skills = opencoder_core::discover_skills();
            let mut resolved_bodies = Vec::new();
            let mut resolved_names = std::collections::HashSet::new();
            for name in &names {
                if let Some(sk) = skills.iter().find(|s| &s.name == name) {
                    resolved_bodies.push(opencoder_core::body_with_source(sk));
                    resolved_names.insert(sk.name.clone());
                }
            }
            if !resolved_bodies.is_empty() {
                let body = resolved_bodies.join("\n\n");
                session.set_skill(Some(body));
                session.set_active_skill_names(resolved_names.clone());
            }
            // Rebuild from the ORIGINAL prompt so only resolved tokens are
            // stripped. extract_skill_tokens strips ALL $name sequences,
            // silently deleting user content for names that matched no skill.
            opencoder_core::strip_resolved_skill_tokens(&prompt, &resolved_names)
        };
        // When skill stripping collapses a compound control command (e.g.
        // "/plan $skill" -> "/plan"), forward the original text so the runner
        // resolves the skill and injects the trigger instead of treating it as
        // a mode-switch-only no-op.
        if !names.is_empty()
            && opencoder_session::parse_control_cmd(&stripped).is_some()
            && raw.trim() != stripped
        {
            raw.trim().to_string()
        } else {
            stripped
        }
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
/// System-prompt replacement body for `--prompt-file`: the user's role text
/// plus the standard bash/task tool preamble. In plan mode the preamble is
/// stripped of the 'build' delegation advertisement first, so a custom
/// prompt never re-introduces what `base_prompt_plan` removes. The user's
/// own body text is left untouched; skill-activated stripping is re-checked
/// per turn in `build_system` (no skill can be active at composition time).
fn compose_custom_prompt(kind: AgentKind, body: &str) -> String {
    let preamble = opencoder_core::tool_preamble();
    let preamble = if opencoder_core::build_delegation_hidden(kind, false) {
        opencoder_core::strip_build_delegation(preamble)
    } else {
        preamble.to_string()
    };
    format!("{}\n\n{}", body.trim(), preamble)
}

/// Read `--prompt-file` and store the composed system prompt on
/// `session.agent.prompt`: the exact read→compose→assign seam `run_headless`
/// (hence `main`) exercises for `--agent plan --prompt-file`. Plan agents
/// strip the 'build' ad from the appended preamble, others keep it whole;
/// `pub` so integration tests drive the real assignment path.
pub fn apply_prompt_file(session: &mut SessionState, prompt_file: &Path) -> Result<()> {
    let body = std::fs::read_to_string(prompt_file)
        .map_err(|e| anyhow!("--prompt-file {}: {e}", prompt_file.display()))?;
    session.agent.prompt = compose_custom_prompt(session.agent.kind, &body);
    Ok(())
}

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
    // A slash command never enters the transcript: a bare command prints no
    // header at all (applied inline, nothing recorded) and a compound echoes
    // only its tail — mirroring what the model actually receives.
    let shown = match opencoder_session::consumed_echo_text(prompt) {
        Some(rest) => rest,
        None => return,
    };
    eprintln!("\n\x1b[1muser\x1b[0m: {}\n", shown.trim_end());
}

/// Copy-paste-ready command to resume a session by id.
fn resume_hint(id: &str) -> String {
    format!("resume with: opencoder -s {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_hint_is_copyable_command() {
        assert_eq!(resume_hint("01ABC"), "resume with: opencoder -s 01ABC");
    }

    #[test]
    fn legacy_sandbox_prefix_rewrites_to_plan() {
        // Compound with irregular spacing (the regression case from the
        // sandbox-mode interlude) collapses to the live plan spelling.
        assert_eq!(
            rewrite_legacy_sandbox_prefix("/sandbox  draft the plan"),
            "/plan draft the plan"
        );
        // Bare switch and leading-whitespace variant.
        assert_eq!(rewrite_legacy_sandbox_prefix("/sandbox"), "/plan");
        assert_eq!(rewrite_legacy_sandbox_prefix("  /sandbox now"), "/plan now");
        // Non-control text passes through untouched.
        assert_eq!(
            rewrite_legacy_sandbox_prefix("explain /sandbox to me"),
            "explain /sandbox to me"
        );
        assert_eq!(
            rewrite_legacy_sandbox_prefix("/sandboxed stuff"),
            "/sandboxed stuff"
        );
        assert_eq!(rewrite_legacy_sandbox_prefix("hello world"), "hello world");
        // The live spelling must never be double-rewritten.
        assert_eq!(
            rewrite_legacy_sandbox_prefix("/plan review"),
            "/plan review"
        );
        // And the rewritten compound is a live plan switch for the runner.
        assert!(matches!(
            opencoder_session::split_control_prefix(&rewrite_legacy_sandbox_prefix("/sandbox draft")),
            Some((opencoder_session::ControlCmd::SwitchAgent(name), Some(rest)))
                if name == "plan" && rest == "draft"
        ));
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

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
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

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
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

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
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
    fn prompt_file_plan_composition_omits_build_delegation() {
        let composed = compose_custom_prompt(AgentKind::Plan, "Plan the migration.\n");
        assert!(!composed.contains(opencoder_core::BUILD_DELEGATION_CLAUSE));
        assert!(!composed.contains("'build'"));
        // User body and the tool preamble survive the strip.
        assert!(composed.starts_with("Plan the migration."));
        assert!(composed.contains("## Tools"));
    }

    #[test]
    fn prompt_file_act_composition_keeps_full_preamble() {
        let composed = compose_custom_prompt(AgentKind::Act, "Act as reviewer.");
        assert!(composed.contains(opencoder_core::BUILD_DELEGATION_CLAUSE));
        assert!(composed.starts_with("Act as reviewer."));
    }
}
