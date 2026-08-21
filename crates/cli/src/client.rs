//! `opencode client`: headless remote client. Resolves a session, posts a
//! prompt to a remote `opencode server`, and streams the result back to stdout
//! by decoding the server's SSE `/events` stream. The client stores nothing
//! locally and calls no LLM — it is a thin shell over the server.

use anyhow::{anyhow, bail, Result};
use opencoder_client::Remote;

use crate::client_ops::client_dispatch_sub;
use crate::client_stream::stream_with_reconnect;
use crate::ClientSub;

/// Everything the `client` subcommand's dispatch needs. Plain data carried by
/// value (no behavior); `client_run` is a pure orchestrator over `Remote`.
pub struct ClientRunOpts {
    pub remote: String,
    pub token: Option<String>,
    pub session: Option<String>,
    pub continue_: bool,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub interrupt: bool,
    pub images: Vec<String>,
    pub prompt: String,
    /// `steer` (default) or `queue`.
    pub delivery: String,
    /// Repeatable `--skill`; the LAST value wins for the triggering run.
    pub skills: Vec<String>,
    pub fork: bool,
    pub compact: bool,
    /// `Some(extra)` when `--handoff` given (`""` when no positional extra).
    pub handoff: Option<String>,
    /// `off | ap | review` (validated client-side before sending).
    pub autopilot: Option<String>,
    /// Requirement annotation; an empty string clears it.
    pub annotation: Option<String>,
    /// Steer a running subagent task; `prompt` is the steer text.
    pub steer_task: Option<String>,
    /// Workdir filter from the global `--workdir` flag (`None` → cwd).
    pub workdir: Option<String>,
    pub cmd: Option<ClientSub>,
}

/// Resolve the client bearer token: `--token` flag, then
/// `OPENCODER_SERVER_TOKEN` env. Unlike the server, the client does NOT
/// auto-generate a token (a random token could never authenticate).
pub fn resolve_token(token: Option<String>) -> Result<String> {
    if let Some(t) = token {
        return Ok(t);
    }
    std::env::var("OPENCODER_SERVER_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow!("no token: pass --token <T> or set OPENCODER_SERVER_TOKEN"))
}

/// Resolve which session id / continue flag the `client` subcommand should use,
/// falling back to the global CLI flags when the subcommand's own flags were
/// not given. Pure extraction of the resolution done in `main.rs`'s Client arm
/// so the shadowing/fallback is unit-testable without a server.
///
/// The `Client` subcommand re-declares its own `--session`/`--continue`, which
/// shadow the globals; without this fallback, `opencode --continue client ...`
/// would silently ignore the global and create a fresh remote session.
pub fn resolve_client_session_flags(
    client_session: Option<String>,
    client_continue: bool,
    global_session: Option<String>,
    global_continue: bool,
) -> (Option<String>, bool) {
    (
        client_session.or(global_session),
        client_continue || global_continue,
    )
}

/// Resolve the workdir used to filter remote sessions (`--continue` resolution
/// and `client session list`): the global `--workdir` flag (the Client variant
/// has no local duplicate — the global is accepted in subcommand position and
/// a client-local shadow could not coexist with its `PathBuf` type), then the
/// process cwd. Falls back to "." when the cwd cannot be read (never blocks
/// the run).
pub fn resolve_client_workdir(global_workdir: Option<&str>) -> String {
    if let Some(w) = global_workdir {
        return w.to_string();
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".into())
}

/// Client-side autopilot validation: only `off | ap | review` are meaningful;
/// anything else is a usage error, not a server round-trip.
pub fn validate_autopilot(mode: &str) -> Result<()> {
    match mode {
        "off" | "ap" | "review" => Ok(()),
        other => bail!("invalid --autopilot {other:?}: expected off | ap | review"),
    }
}

pub async fn client_run(opts: ClientRunOpts) -> Result<()> {
    let token = resolve_token(opts.token)?;
    let client = Remote::new(&opts.remote, &token)?;
    let workdir = resolve_client_workdir(opts.workdir.as_deref());

    // Subcommand path wins over the trailing prompt.
    if let Some(sub) = opts.cmd {
        return client_dispatch_sub(&client, sub, &workdir).await;
    }

    let has_prompt = !opts.prompt.trim().is_empty();
    let needs_existing_session = opts.interrupt
        || opts.fork
        || opts.compact
        || opts.handoff.is_some()
        || opts.steer_task.is_some()
        || opts.autopilot.is_some()
        || opts.annotation.is_some();
    if !has_prompt && !needs_existing_session {
        bail!(
            "no prompt provided. Usage: opencode client -r <URL> \"your prompt\"  |  \
             opencode client -r <URL> --compact --continue"
        );
    }
    if opts.steer_task.is_some() && !has_prompt {
        bail!("--steer-task requires the steer text as the prompt");
    }

    // Resolve the target session: explicit id > --continue (most recent in the
    // workdir) > create a fresh one (only for a plain prompt run). Management
    // ops (interrupt/fork/compact/handoff/steer/autopilot/annotation) never
    // create a session — operating on one that doesn't exist yet is
    // meaningless, so require an explicit resolution.
    let session_id = if let Some(id) = opts.session {
        id
    } else if opts.continue_ {
        let list = client.list_sessions(None, None, Some(&workdir)).await?;
        list.first()
            .and_then(|item| item.get("id"))
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                anyhow!("no sessions on the server for workdir {workdir}; pass --workdir <path> or --session <id>")
            })?
    } else if needs_existing_session {
        bail!("--interrupt/--fork/--compact/--handoff/--steer-task/--autopilot/--annotation require --session <id> or --continue");
    } else {
        client
            .create_session(opts.agent.as_deref(), opts.model.as_deref())
            .await?
    };

    // --fork: copy the resolved session and operate on the copy from here on.
    let session_id = if opts.fork {
        let new_id = client.fork_session(&session_id).await?;
        eprintln!("\x1b[2m[forked {session_id} -> {new_id}]\x1b[0m");
        new_id
    } else {
        session_id
    };

    // Session-scoped settings, applied before the prompt (or standalone when
    // no prompt follows: the flags then act as a "configure and exit" op).
    if let Some(mode) = opts.autopilot.as_deref() {
        validate_autopilot(mode)?;
        client.post_autopilot(&session_id, Some(mode)).await?;
    }
    if let Some(text) = opts.annotation.as_deref() {
        client
            .post_annotation(&session_id, if text.is_empty() { None } else { Some(text) })
            .await?;
    }

    if opts.interrupt {
        // Structured verdict: HTTP 200 carries {"ok":false,"error":...} when
        // there was nothing draining — that is a failure for scripts.
        let v = client.interrupt(&session_id).await?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("interrupt failed");
            bail!("{err}");
        }
        eprintln!(
            "\n\x1b[2m[interrupted remote session {}]\x1b[0m",
            session_id
        );
        return Ok(());
    }

    if opts.compact {
        client.post_compact(&session_id).await?;
        eprintln!(
            "\x1b[2m[compaction queued for remote session {}]\x1b[0m",
            session_id
        );
        return Ok(());
    }

    if let Some(extra) = opts.handoff.as_deref() {
        client.post_handoff(&session_id, extra).await?;
        eprintln!(
            "\x1b[2m[plan handoff submitted for remote session {}]\x1b[0m",
            session_id
        );
        return Ok(());
    }

    if !has_prompt {
        // Settings-only run (--autopilot/--annotation without a prompt).
        eprintln!("\x1b[2m[remote session {} updated]\x1b[0m", session_id);
        return Ok(());
    }

    // Snapshot the current event cursor so we only stream events produced by
    // THIS prompt (not the whole prior transcript).
    let after = client.last_event_seq(&session_id).await?;

    let image_uris = crate::run_image::load_image_data_uris(&opts.images)?;
    let skill = opts.skills.last().cloned();

    if let Some(task_id) = opts.steer_task.as_deref() {
        eprintln!(
            "\n\x1b[1msteer {task_id}\x1b[0m: {}\n",
            opts.prompt.trim_end()
        );
        client
            .steer_subagent(&session_id, task_id, &opts.prompt, &image_uris)
            .await?;
    } else {
        eprintln!("\n\x1b[1muser\x1b[0m: {}\n", opts.prompt.trim_end());
        client
            .post_prompt(
                &session_id,
                &opts.prompt,
                Some(opts.delivery.as_str()),
                skill.as_deref(),
                opts.agent.as_deref(),
                opts.model.as_deref(),
                &image_uris,
            )
            .await?;
    }

    stream_with_reconnect(&client, &session_id, after).await?;
    eprintln!("\n\x1b[2m[remote session {}]\x1b[0m", session_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ClientRunOpts;
    use super::{
        resolve_client_session_flags, resolve_client_workdir, resolve_token, validate_autopilot,
    };

    #[test]
    fn resolve_token_param_returns_ok() {
        assert_eq!(resolve_token(Some("explicit".into())).unwrap(), "explicit");
    }

    #[test]
    fn client_session_flags_fall_back_to_globals() {
        // Client's own --session wins over the global.
        let (s, _) = resolve_client_session_flags(
            Some("c-sess".into()),
            false,
            Some("g-sess".into()),
            false,
        );
        assert_eq!(s.as_deref(), Some("c-sess"));

        // No client --session -> global is used (otherwise it would be lost).
        let (s, _) = resolve_client_session_flags(None, false, Some("g-sess".into()), false);
        assert_eq!(s.as_deref(), Some("g-sess"));

        // Neither set -> None.
        let (s, _) = resolve_client_session_flags(None, false, None, false);
        assert!(s.is_none());
    }

    #[test]
    fn client_continue_flags_or_with_globals() {
        // client flag alone
        let (_, c) = resolve_client_session_flags(None, true, None, false);
        assert!(c);
        // global alone (this is the bug: previously shadowed & dropped)
        let (_, c) = resolve_client_session_flags(None, false, None, true);
        assert!(c);
        // neither
        let (_, c) = resolve_client_session_flags(None, false, None, false);
        assert!(!c);
    }

    #[test]
    fn workdir_prefers_global_flag_then_cwd() {
        assert_eq!(resolve_client_workdir(Some("/b")), "/b");
        // No flag -> cwd (deterministic inside a process).
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(resolve_client_workdir(None), cwd);
    }

    #[test]
    fn autopilot_accepts_known_modes_and_rejects_others() {
        for ok in ["off", "ap", "review"] {
            assert!(validate_autopilot(ok).is_ok(), "{ok} must be valid");
        }
        for bad in ["auto", "", "AP", "review!"] {
            assert!(validate_autopilot(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn client_opts_is_plain_data() {
        // Construction smoke: the opts struct carries every Client flag so the
        // dispatch cannot silently drop one.
        let opts = ClientRunOpts {
            remote: "http://x".into(),
            token: None,
            session: None,
            continue_: false,
            agent: None,
            model: None,
            interrupt: false,
            images: vec![],
            prompt: String::new(),
            delivery: "steer".into(),
            skills: vec![],
            fork: false,
            compact: false,
            handoff: None,
            autopilot: None,
            annotation: None,
            steer_task: None,
            workdir: None,
            cmd: None,
        };
        assert_eq!(opts.remote, "http://x");
        assert_eq!(opts.delivery, "steer");
    }
}
