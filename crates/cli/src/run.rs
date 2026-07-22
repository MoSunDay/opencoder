use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatClient, ChatStream};
use opencoder_session::{
    generate_title, resume_and_replay as resume_session, run_once, SessionEvent, SessionState,
};
use opencoder_store::{SessionFilter, Store};

use crate::Cli;

pub async fn run_headless(cli: &Cli, prompt: String) -> Result<()> {
    let workdir = resolve_workdir(cli)?;
    let config = Config::load(&workdir)?;
    let ep = config.resolve_endpoint()?;
    let client: Arc<dyn ChatStream> = Arc::new(
        ChatClient::new(&ep.base_url, &ep.api_key, &ep.headers, config.network.proxy.as_deref())?,
    );
    let store: Option<Arc<dyn Store>> = crate::session_cmd::open_store(&workdir)
        .await
        .ok()
        .map(|s| Arc::new(s) as Arc<dyn Store>);

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
            None,
        )
        .await?
    } else {
        let agent_name = config.agent.default.as_str();
        let agent = resolve_agent(agent_name)
            .or_else(|| resolve_agent("act"))
            .ok_or_else(|| anyhow!("agent not found: {agent_name}"))?;
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

    if session.store.is_none() {
        if let Some(st) = &store {
            session.store = Some(st.clone());
        }
    }

    print_resume_summary(&session).await;

    if let Some(pf) = &cli.prompt_file {
        let body = std::fs::read_to_string(pf)
            .map_err(|e| anyhow!("--prompt-file {}: {e}", pf.display()))?;
        session.agent.prompt =
            format!("{}\n\n{}", body.trim(), opencoder_core::tool_preamble());
    }

    // Extract and resolve {$skill-name} tokens from the prompt.
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
    opencoder_session::run(&mut session, prompt, |ev| print_event(&ev)).await?;

    // cheap background title generation (small model) after the first round
    generate_title(&session).await;

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

pub(crate) fn print_event(ev: &SessionEvent) {
    match ev {
        SessionEvent::TextDelta(t) => {
            print!("{t}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        SessionEvent::ReasoningDelta(_) => {}
        SessionEvent::ToolStart { name, input, .. } => {
            if name == "task" {
                return;
            }
            eprintln!(
                "\n\x1b[36m\u{25b8} {name}\x1b[0m {}",
                summarize_input(input)
            );
        }
        SessionEvent::ToolEnd {
            name,
            output,
            is_error,
            ..
        } => {
            let color = if *is_error { "31" } else { "2" };
            eprintln!("\x1b[{color}m  {}\x1b[0m", indent_first(output, 2));
            let _ = name;
        }
        SessionEvent::AgentSwitch(to) => {
            eprintln!("\n\x1b[35m[switched to {to} mode]\x1b[0m");
        }
        SessionEvent::Compaction(s) => {
            eprintln!("\n\x1b[33m[context compacted]\x1b[0m {}", truncate(s, 160));
        }
        SessionEvent::Status(s) => {
            eprintln!("\x1b[2m[{s}]\x1b[0m");
        }
        SessionEvent::Done => {
            println!("\n");
        }
        SessionEvent::Error(e) => {
            eprintln!("\n\x1b[31merror: {e}\x1b[0m");
        }
        SessionEvent::SubagentStart { kind, prompt, .. } => {
            eprintln!("\x1b[34m\u{2937} subagent [{kind}] {prompt}\x1b[0m");
        }
        SessionEvent::SubagentEnd { ok, summary, .. } => {
            let mark = if *ok { "\u{2714}" } else { "\u{2718}" };
            eprintln!("\x1b[34m  {mark} {summary}\x1b[0m");
        }
        SessionEvent::PlanHandoff(plan) => {
            eprintln!("\n\x1b[33m\u{2500}\u{2500} plan \u{2500}\u{2500}\x1b[0m\n{plan}\n");
        }
        SessionEvent::TranscriptReset(_) => {}
        SessionEvent::QueueConsumed { .. } => {}
        SessionEvent::SteerConsumed { .. } => {}
        SessionEvent::SubagentChild { .. } => {}
    }
}

fn summarize_input(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(map) => {
            if let Some(c) = map.get("command").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            if let Some(c) = map.get("path").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            if let Some(c) = map.get("description").and_then(|v| v.as_str()) {
                return truncate(c, 100);
            }
            let s = serde_json::to_string(input).unwrap_or_default();
            truncate(&s, 100)
        }
        other => {
            let s = serde_json::to_string(other).unwrap_or_default();
            truncate(&s, 100)
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        let cut: String = t.chars().take(n).collect();
        format!("{cut}...")
    }
}

fn indent_first(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
pub fn _duration() -> Duration {
    Duration::from_secs(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_input_extracts_command() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(summarize_input(&input), "ls -la");
    }

    #[test]
    fn summarize_input_extracts_path_when_no_command() {
        let input = serde_json::json!({"path": "/tmp/foo.rs"});
        assert_eq!(summarize_input(&input), "/tmp/foo.rs");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let long = "a".repeat(120);
        let t = truncate(&long, 10);
        assert!(t.ends_with("..."));
        assert_eq!(t.chars().count(), 13); // 10 + "..."
    }

    #[test]
    fn truncate_short_returns_as_is() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn indent_first_pads_each_line() {
        let s = "line1\nline2";
        assert_eq!(indent_first(s, 2), "  line1\n  line2");
    }

    #[test]
    fn resume_hint_is_copyable_command() {
        assert_eq!(
            resume_hint("01ABC"),
            "resume with: opencoder -s 01ABC"
        );
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
            task("t1", "explore", "find all TODO comments", SubagentStatus::Completed, Some(true)),
            task("t2", "build", "fix the bug in module foo bar baz qux", SubagentStatus::Failed, Some(false)),
        ];
        let s = format_resume_summary(&tasks).expect("non-empty -> Some");
        assert!(s.contains("2/2 subagents done"), "got: {s}");
        assert!(s.contains('\u{2714}'), "completed mark (✔) present: {s}");
        assert!(s.contains('\u{2718}'), "failed mark (✘) present: {s}");
        assert!(s.contains("find all TODO comments"), "explore prompt present: {s}");
        assert!(s.contains("fix the bug in module foo bar baz qux"), "build prompt present: {s}");

        // A Running task counts toward total but not done.
        let running = vec![task("r", "explore", "still going", SubagentStatus::Running, None)];
        let s = format_resume_summary(&running).expect("Some");
        assert!(s.contains("0/1 subagents done"), "running not counted as done: {s}");
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
        let resolved =
            pick_resume_id(&cli, Some(&store as &dyn Store)).await.unwrap();
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
            })
            .await
            .unwrap();

        // `--session <session_id>` should be returned unchanged.
        let cli = Cli::parse_from(["opencoder", "--session", session_id]);
        let resolved =
            pick_resume_id(&cli, Some(&store as &dyn Store)).await.unwrap();
        assert_eq!(resolved.as_deref(), Some(session_id));
    }
}
