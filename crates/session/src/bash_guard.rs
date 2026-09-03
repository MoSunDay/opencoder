//! Bash command write-detection for read-only session enforcement (plan
//! mode and the sidecar bypass loop).
//!
//! In plan mode the agent must not modify the system, and the sidecar — a
//! temporary Q&A loop over a snapshot of the main session — shares that
//! invariant. Rather than removing `bash` entirely (it's useful for `ls`,
//! `cat`, `grep`, `find`), we classify each command as read-only or
//! potentially-mutating and block the latter.
//!
//! The classifier is now a thin adapter over the [`opencoder-shellguard`]
//! crate, which is derived from [rippy](https://github.com/mpecan/rippy)
//! (MIT license, copyright the rippy authors). Plan/sidecar policy: block all
//! risk-bearing writes. Shellguard still identifies sandbox-released `/tmp`
//! writes, but the read-only sessions reject those too: only non-persistent
//! device/fd redirects remain harmless. `Ask`/`Deny` and any state-writing
//! `Allow` verdict block.
//!
//! Classification is cwd-relative (a bare `touch f` means "write `f` in the
//! working directory"), so the cwd handed to [`classify_with_dir`] MUST be
//! the directory the command will actually execute in — the per-call
//! `workdir` input for bash, not the agent process's cwd. Classifying
//! against any other directory is the B2 bypass: from a process cwd under
//! `/tmp` a released verdict lets the write land in the real workdir.
//!
//! The command-parsing helpers below (`cmd_base`, `strip_wrappers` and their
//! private support fns) are preserved verbatim from the previous hand-written
//! classifier: [`crate::tools::ssh_pty`] reuses them to unwrap
//! privilege-escalators before running commands on remote hosts.

/// Verdict on whether a bash command may modify state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashVerdict {
    /// Command appears read-only; safe to execute in plan mode.
    ReadOnly,
    /// Command is blocked; carries a human-readable reason shown to the model.
    WriteBlocked(String),
}

/// Classify a bash command for plan-mode enforcement by delegating to
/// `opencoder_shellguard::classify` (derived from rippy, MIT). `Allow`
/// passes; `Ask`/`Deny` block, carrying the classifier's human-readable
/// reason (embedded verbatim in the tool error shown to the model).
///
/// Relative paths resolve against the *process* cwd. Production gating must
/// use [`classify_with_dir`] with the directory the command will run in:
/// the classification cwd must equal the execution cwd.
pub fn classify(command: &str) -> BashVerdict {
    map_verdict(opencoder_shellguard::classify(command))
}

/// Map shellguard's sandbox verdict to the stricter plan contract. A write
/// under `/tmp` is an allowed sandbox effect but is still a write, so plan
/// mode blocks it using the typed `writes_state` provenance.
fn map_verdict(verdict: opencoder_shellguard::Verdict) -> BashVerdict {
    use opencoder_shellguard::Decision;
    match verdict.decision {
        Decision::Allow if !verdict.writes_state => BashVerdict::ReadOnly,
        Decision::Allow => BashVerdict::WriteBlocked(verdict.reason),
        Decision::Ask | Decision::Deny => BashVerdict::WriteBlocked(verdict.reason),
    }
}

/// [`classify`] against an explicit working directory: relative operands
/// (`touch f`) resolve as if the shell were running in `cwd`. The caller
/// must pass the exact directory the command will execute in — for the bash
/// tool that is the per-call `workdir` input, defaulting to the session
/// working dir (see `tools::bash`).
pub fn classify_with_dir(command: &str, cwd: &std::path::Path) -> BashVerdict {
    map_verdict(opencoder_shellguard::classify_in(command, cwd))
}

/// Tool names a plan session may execute, mirroring the `plan` agent's
/// `ToolFilter::Allow` list. `question` still has to clear the latent-skill
/// gate downstream; `bash` additionally passes the shellguard classifier.
const PLAN_ADMITTED: &[&str] = &["bash", "task", "question"];

/// Tool names a sidecar session may execute, mirroring the sidecar agent's
/// `ToolFilter::Allow` list. Read-only inspection only: `bash` additionally
/// passes the shellguard classifier.
const SIDECAR_ADMITTED: &[&str] = &["read", "search", "ls", "bash"];

/// Canonical model-facing denial for a plan-mode interception (candidate
/// wording, verbatim). The contract: name the mode, state the read-only
/// invariant, tell the model to stop implementation attempts, route context
/// gathering to a read-only `explore` subagent instead of bash, focus on a
/// plan, and point at the real escape hatch — switch to act via `/agent act`.
pub fn plan_denial(tool: &str, detail: &str) -> String {
    if tool == "bash" {
        format!(
            "Blocked in plan mode (read-only): this bash command modifies state ({detail}) \
             and was not executed. Do not retry or attempt another write. To gather context, \
             delegate read-only investigation to an 'explore' subagent (task tool) instead \
             of bash. Focus on analysis and output a plan only; do not execute implementation. \
             To make changes, switch to the act agent (/agent act)."
        )
    } else {
        format!(
            "Blocked in plan mode (read-only): `{tool}` was not executed - {detail}. \
             Do not retry or attempt another write. To gather context, delegate read-only \
             investigation to an 'explore' subagent (task tool). Focus on analysis and \
             output a plan only; do not execute implementation. To make changes, switch \
             to the act agent (/agent act)."
        )
    }
}

/// Canonical model-facing denial for a sidecar interception. Same shape as
/// [`plan_denial`] but scoped to the read-only Q&A loop: name the session,
/// state the invariant, forbid retries and alternate write paths, and point
/// at the real escape hatch — the main session.
pub fn sidecar_denial(tool: &str, detail: &str) -> String {
    if tool == "bash" {
        format!(
            "Blocked in sidecar (read-only Q&A): this bash command modifies state ({detail}) \
             and was not executed. Do not retry or attempt another write. Gather what you need \
             with read-only commands (read, search, git log) or answer directly from the snapshot \
             context. This sidecar cannot make changes; to make changes, return to the main session."
        )
    } else {
        format!(
            "Blocked in sidecar (read-only Q&A): `{tool}` was not executed - {detail}. \
             Do not retry or attempt another write. Answer from the snapshot context; \
             read-only inspection only. To make changes, return to the main session."
        )
    }
}

/// Admitted-tool table plus denial context: (allowed tools, reason for an
/// unadmitted tool, per-tool denial message builder). Both tables are
/// compile-time constants, hence 'static.
type GateRule = (
    &'static [&'static str],
    &'static str,
    fn(&str, &str) -> String,
);

/// Read-only execution gate for one tool call: `Some(denial)` refuses the
/// call with the model-visible [`plan_denial`] / [`sidecar_denial`], `None`
/// lets it proceed. Gated sessions: plan mode and the sidecar loop; every
/// other session (including other `Subagent`-kind agents) passes through.
///
/// `workdir` is the directory the call will execute in — for bash the
/// per-call `workdir` input, else the session working dir (resolved by the
/// caller exactly like `tools::bash` does). The bash branch classifies the
/// command against it, because the classification cwd must equal the
/// execution cwd: a relative write judged against any other directory is
/// the B2 conditional-bypass (a process cwd under `/tmp` would release
/// `touch f` while the write lands in the real workdir).
///
/// Two layers, both fail-closed:
/// - the session is read-only but the tool is not admitted (a hallucinated or
///   remembered builtin like `edit`, or an unadvertised MCP tool): refuse so
///   a write can never slip through a tool the model was never shown;
/// - `bash` is admitted but the shellguard classifier flags the command as
///   mutating: refuse with the classifier's reason.
pub fn gate(
    kind: &opencoder_core::AgentKind,
    agent_name: &str,
    tool: &str,
    command: Option<&str>,
    workdir: &std::path::Path,
) -> Option<String> {
    let (admitted, unadmitted_detail, denial): GateRule =
        if *kind == opencoder_core::AgentKind::Plan {
            (
                PLAN_ADMITTED,
                "this tool is not available in plan mode",
                plan_denial,
            )
        } else if agent_name == "sidecar" {
            (
                SIDECAR_ADMITTED,
                "this tool is not available in the sidecar",
                sidecar_denial,
            )
        } else {
            return None;
        };
    if tool != "bash" && !admitted.contains(&tool) {
        return Some(denial(tool, unadmitted_detail));
    }
    if tool == "bash" {
        if let BashVerdict::WriteBlocked(reason) = classify_with_dir(command.unwrap_or(""), workdir)
        {
            return Some(denial("bash", &reason));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Command-parsing helpers (shared with `tools::ssh_pty`).
//
// These unwrap privilege-escalators and command wrappers so a write command is
// never mistaken for read-only just because it was prefixed with `env`,
// `nohup`, `timeout`, `nice`, `command`, `strace`, …
// ---------------------------------------------------------------------------

/// Strip a *leading* `sudo`/`doas` prefix (recursively, so `sudo sudo rm`
/// fully unwraps). Returns a slice of `s`; does not allocate.
pub(crate) fn strip_leading_sudo(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed
        .strip_prefix("sudo ")
        .or_else(|| trimmed.strip_prefix("doas "))
    {
        strip_leading_sudo(rest)
    } else {
        trimmed
    }
}

/// Returns true when `tok` looks like an environment-variable assignment
/// (`KEY=value`) that `env` consumes before the real command.
fn is_env_assignment(tok: &str) -> bool {
    if let Some(eq) = tok.find('=') {
        let key = &tok[..eq];
        !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    } else {
        false
    }
}

/// Whether `tok` is an option token (`-i`, `-n5`, `--long`, `-`, `--`).
/// Used to skip a wrapper's own flags so the wrapped command is revealed
/// (`env -i rm x`, `stdbuf -o0 rm x`). The bare `-` (stdin convention,
/// e.g. `python -`) is treated as an option on purpose.
fn is_option_token(tok: &str) -> bool {
    tok.starts_with('-')
}

/// Skip leading option tokens of a wrapped command. Plain options (`-i`,
/// `--long`, `-n5` with the value attached) are dropped; options listed in
/// `valued` take their value as a *separate* token (`nice -n 5 rm x`) and
/// consume it too. Returns the remainder starting at the first non-option
/// token.
fn skip_option_tokens<'a>(cmd: &'a str, valued: &[&str]) -> &'a str {
    let mut rest = cmd.trim_start();
    while let Some(tok) = rest.split_whitespace().next() {
        if !is_option_token(tok) {
            break;
        }
        rest = rest[tok.len()..].trim_start();
        if valued.contains(&tok) {
            if let Some(val) = rest.split_whitespace().next() {
                rest = rest[val.len()..].trim_start();
            }
        }
    }
    rest
}

/// The trailing path component of the command's first token — e.g.
/// `/usr/bin/rm -rf x` → `rm`. Returns `""` for empty/whitespace input.
pub(crate) fn cmd_base(cmd: &str) -> &str {
    let first = cmd.split_whitespace().next().unwrap_or("");
    first.rsplit('/').next().unwrap_or(first)
}

/// Strip wrapper commands (`env`, `exec`, `command`, `nohup`, `timeout`,
/// `strace`, `ltrace`, `perf`, `valgrind`, `nice`, `ionice`, `time`,
/// `stdbuf`, `setsid`) — and a leading `sudo`/`doas` — that merely delegate
/// to the real program. This prevents trivial guard bypasses like `env rm`,
/// `exec rm`, or `nohup rm`.
///
/// Applies recursively so `env VAR=x exec sudo rm` is fully unwrapped.
/// Wrapper-specific handling:
/// - `env`: skips leading `KEY=value` assignments *and* option tokens
///   (`env -i rm x`, `env -u FOO VAR=1 rm x`).
/// - `nice`/`ionice`: skips options; `-n`/`-c`/`-p` also consume the separate
///   value token (`nice -n 5 rm x`, `ionice -c 2 rm x`).
/// - `timeout`: skips options (`-k`/`-s` consume a value), then one duration
///   token (`timeout -k 1 5 rm x`).
/// - `time`/`stdbuf`/`setsid` and the tracing tools (`strace`/`ltrace`/
///   `perf`/`valgrind`): skip leading option tokens (`stdbuf -o0 rm x`).
pub(crate) fn strip_wrappers(cmd: &str) -> &str {
    let stripped = strip_leading_sudo(cmd);
    let first = stripped.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    let rest = stripped[first.len()..].trim_start();

    match base {
        "env" => {
            // `env` accepts options (`-u NAME` takes a separate value) and
            // `KEY=value` assignments (in any order) before the real command.
            let mut pos = rest;
            while let Some(tok) = pos.split_whitespace().next() {
                if is_option_token(tok) {
                    pos = pos[tok.len()..].trim_start();
                    if matches!(tok, "-u" | "--unset") {
                        if let Some(name) = pos.split_whitespace().next() {
                            pos = pos[name.len()..].trim_start();
                        }
                    }
                } else if is_env_assignment(tok) {
                    pos = pos[tok.len()..].trim_start();
                } else {
                    break;
                }
            }
            strip_wrappers(pos)
        }
        // Wrappers without options of their own.
        "exec" | "command" | "nohup" => strip_wrappers(rest),
        "nice" => strip_wrappers(skip_option_tokens(rest, &["-n", "--adjustment"])),
        "ionice" => strip_wrappers(skip_option_tokens(rest, &["-c", "-n", "-p"])),
        "time" | "stdbuf" | "setsid" => strip_wrappers(skip_option_tokens(rest, &[])),
        "timeout" => {
            // After its options, `timeout` takes one duration token before
            // the real command (`timeout -k 1 5 rm x`).
            let after_flags = skip_option_tokens(rest, &["-k", "-s", "--kill-after", "--signal"]);
            let after_duration = match after_flags.split_whitespace().next() {
                Some(duration) => after_flags[duration.len()..].trim_start(),
                None => after_flags,
            };
            strip_wrappers(after_duration)
        }
        "strace" | "ltrace" | "perf" | "valgrind" => strip_wrappers(skip_option_tokens(rest, &[])),
        _ => stripped,
    }
}

/// A workdir OUTSIDE the /tmp release scope. The crate tree itself may sit
/// under /tmp (which the shellguard releases wholesale), so tests that need
/// a *plain* directory must not anchor on CARGO_MANIFEST_DIR or the process
/// cwd.
#[cfg(test)]
pub(crate) fn plain_dir() -> tempfile::TempDir {
    let home = std::env::var("HOME").expect("$HOME set");
    tempfile::Builder::new()
        .prefix("sg-plain-")
        .tempdir_in(home)
        .expect("writable $HOME for a non-released workdir")
}

#[cfg(test)]
#[path = "bash_guard_compat_tests.rs"]
mod compat_tests;

#[cfg(test)]
#[path = "bash_guard_compat_tests2.rs"]
mod compat_tests2;

#[cfg(test)]
mod tests {
    use super::{classify, cmd_base, strip_wrappers, BashVerdict};

    #[test]
    fn read_only_passes() {
        assert_eq!(classify("ls -la"), BashVerdict::ReadOnly);
    }

    #[test]
    fn tmp_write_is_blocked_by_strict_plan_policy() {
        assert!(matches!(
            classify("echo x > /tmp/a.log"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn destructive_command_blocked_with_reason() {
        match classify("rm -rf /var/x") {
            BashVerdict::WriteBlocked(reason) => assert!(!reason.is_empty()),
            other => panic!("expected WriteBlocked, got {other:?}"),
        }
    }

    #[test]
    fn cwd_is_not_released() {
        assert!(matches!(
            classify("echo x > ./f"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn script_execution_blocked() {
        assert!(matches!(
            classify("bash /tmp/x.sh"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn strip_wrappers_reveals_env_wrapped_command() {
        assert!(strip_wrappers("env rm -rf /").starts_with("rm"));
    }

    #[test]
    fn cmd_base_takes_trailing_component() {
        assert_eq!(cmd_base("/usr/bin/rm"), "rm");
    }

    #[test]
    fn denial_names_mode_forbids_retry_points_at_act() {
        let msg = super::plan_denial("edit", "this tool is not available in plan mode");
        assert!(msg.contains("Blocked in plan mode"), "got: {msg}");
        assert!(msg.contains("read-only"), "got: {msg}");
        assert!(msg.contains("`edit`"), "got: {msg}");
        assert!(msg.contains("output a plan only"), "got: {msg}");
        assert!(msg.contains("Do not retry"), "got: {msg}");
        // Context gathering must be routed to the read-only explore
        // subagent, not retried through bash.
        assert!(msg.contains("'explore' subagent"), "got: {msg}");
        // The escape hatch must name the real path: the act agent.
        assert!(
            msg.contains("switch to the act agent (/agent act)"),
            "got: {msg}"
        );
    }

    #[test]
    fn bash_denial_routes_context_gathering_to_explore() {
        let msg = super::plan_denial("bash", "cd to /etc");
        assert!(msg.contains("'explore' subagent (task tool)"), "got: {msg}");
        assert!(msg.contains("instead of bash"), "got: {msg}");
        assert!(msg.contains("Do not retry"), "got: {msg}");
    }

    #[test]
    fn cd_navigation_is_read_only() {
        // `cd` writes nothing and the analyzer re-aims its analysis cwd, so
        // navigation must not trip the plan gate (previously it asked).
        for cmd in [
            "cd src",
            "cd ..",
            "cd /etc",
            "cd -P src",
            "cd -- /var",
            "cd /tmp && ls",
        ] {
            assert_eq!(
                classify(cmd),
                BashVerdict::ReadOnly,
                "{cmd} must classify as read-only"
            );
        }
    }

    #[test]
    fn cd_does_not_weaken_write_detection_after_it() {
        // The destination re-aim must keep later operands judgeable: a write
        // after `cd` is still blocked, with the write (not the cd) as reason.
        for cmd in [
            "cd /tmp && touch f",
            "cd src && touch f",
            "cd .. && rm -rf x",
        ] {
            assert!(
                matches!(classify(cmd), BashVerdict::WriteBlocked(_)),
                "{cmd} must stay blocked"
            );
        }
        // Unresolvable destinations stay fail-closed (`$HOME` expands
        // statically to a literal path, so it is judgeable and allowed).
        for cmd in ["cd $UNSET_VAR_XYZ", "cd ~", "cd"] {
            assert!(
                matches!(classify(cmd), BashVerdict::WriteBlocked(_)),
                "{cmd} must stay blocked (unresolvable destination)"
            );
        }
    }

    #[test]
    fn admitted_set_matches_plan_agent_tool_filter() {
        let plan = opencoder_core::resolve_agent("plan").expect("plan agent");
        for name in super::PLAN_ADMITTED {
            assert!(
                plan.tools.allows(name),
                "gate admits {name} but the plan ToolFilter does not"
            );
        }
    }

    #[test]
    fn admitted_set_matches_sidecar_agent_tool_filter() {
        let sidecar = opencoder_core::resolve_agent("sidecar").expect("sidecar agent");
        for name in super::SIDECAR_ADMITTED {
            assert!(
                sidecar.tools.allows(name),
                "gate admits {name} but the sidecar ToolFilter does not"
            );
        }
    }

    #[test]
    fn gate_passes_non_plan_kinds_through() {
        use opencoder_core::{resolve_agent, AgentKind};
        let anywhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["act", "explore"] {
            let agent = resolve_agent(name).unwrap();
            assert_eq!(
                super::gate(&agent.kind, &agent.name, "edit", Some("x"), anywhere),
                None,
                "{name} must not be gated"
            );
        }
        // Explicit non-plan kind is equally untouched.
        assert_eq!(
            super::gate(&AgentKind::Act, "act", "bash", Some("rm -rf /"), anywhere),
            None
        );
    }

    #[test]
    fn gate_refuses_unadmitted_tool_in_plan() {
        use opencoder_core::resolve_agent;
        let agent = resolve_agent("plan").unwrap();
        let anywhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for tool in ["edit", "bg", "mcp__fs__write"] {
            let denial = super::gate(&agent.kind, &agent.name, tool, None, anywhere)
                .unwrap_or_else(|| panic!("{tool} must be refused"));
            assert!(denial.contains("Blocked in plan mode"), "got: {denial}");
        }
        // Admitted tools pass the first layer untouched.
        for tool in super::PLAN_ADMITTED {
            if *tool == "bash" {
                continue; // covered by the classifier tests below
            }
            assert_eq!(
                super::gate(&agent.kind, &agent.name, tool, None, anywhere),
                None,
                "{tool} admitted"
            );
        }
    }

    #[test]
    fn gate_blocks_mutating_bash_in_plan() {
        use opencoder_core::resolve_agent;
        let agent = resolve_agent("plan").unwrap();
        // A plain (non-released) workdir.
        let plain_dir = super::plain_dir();
        let plain = plain_dir.path();
        assert!(super::gate(&agent.kind, &agent.name, "bash", Some("ls -la"), plain).is_none());
        let denial = super::gate(&agent.kind, &agent.name, "bash", Some("rm -rf ./f"), plain)
            .expect("blocked");
        assert!(denial.contains("Blocked in plan mode"), "got: {denial}");
    }

    #[test]
    fn gate_blocks_writes_in_released_and_plain_call_workdirs() {
        use opencoder_core::resolve_agent;
        let agent = resolve_agent("plan").unwrap();
        // The command is identical in both legs. Shellguard reports different
        // provenance, but strict plan policy blocks both write effects.
        let tmp = std::path::Path::new("/tmp");
        let tmp_denial = super::gate(&agent.kind, &agent.name, "bash", Some("touch ./f"), tmp)
            .expect("relative write under /tmp is still a write in plan mode");
        assert!(tmp_denial.contains("output a plan only"));
        let plain = super::plain_dir();
        let denial = super::gate(
            &agent.kind,
            &agent.name,
            "bash",
            Some("touch ./f"),
            plain.path(),
        )
        .unwrap_or_else(|| panic!("same command must be blocked from a plain workdir"));
        assert!(denial.contains("Blocked in plan mode"), "got: {denial}");
    }

    #[test]
    fn gate_blocks_mutating_bash_in_sidecar() {
        use opencoder_core::resolve_agent;
        let agent = resolve_agent("sidecar").unwrap();
        // The sidecar is a Subagent-kind session: gating keys on the agent
        // name, so a bare AgentKind check would let writes through.
        assert_eq!(agent.kind, opencoder_core::AgentKind::Subagent);
        let plain_dir = super::plain_dir();
        let plain = plain_dir.path();
        assert!(super::gate(&agent.kind, &agent.name, "bash", Some("ls -la"), plain).is_none());
        assert!(super::gate(
            &agent.kind,
            &agent.name,
            "bash",
            Some("git log --oneline -5"),
            plain
        )
        .is_none());
        for cmd in ["rm -rf ./f", "touch ./f"] {
            let denial = super::gate(&agent.kind, &agent.name, "bash", Some(cmd), plain)
                .unwrap_or_else(|| panic!("{cmd} must be blocked"));
            assert!(denial.contains("Blocked in sidecar"), "got: {denial}");
        }
    }

    #[test]
    fn gate_refuses_unadmitted_tool_in_sidecar() {
        use opencoder_core::resolve_agent;
        let agent = resolve_agent("sidecar").unwrap();
        let anywhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for tool in ["edit", "task", "question"] {
            let denial = super::gate(&agent.kind, &agent.name, tool, None, anywhere)
                .unwrap_or_else(|| panic!("{tool} must be refused"));
            assert!(denial.contains("Blocked in sidecar"), "got: {denial}");
        }
        // Admitted read-only tools pass the first layer untouched.
        for tool in ["read", "search", "ls"] {
            assert_eq!(
                super::gate(&agent.kind, &agent.name, tool, None, anywhere),
                None,
                "{tool} admitted"
            );
        }
    }

    #[test]
    fn classify_with_dir_resolves_relative_paths_against_the_given_cwd() {
        use super::classify_with_dir;
        // Shellguard marks /tmp as sandbox-released, but the plan adapter
        // preserves the typed write effect and refuses it.
        assert!(matches!(
            classify_with_dir("touch f", std::path::Path::new("/tmp")),
            BashVerdict::WriteBlocked(_)
        ));
        let plain = super::plain_dir();
        assert!(matches!(
            classify_with_dir("touch f", plain.path()),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn helpers_are_idempotent_on_plain_commands() {
        let plain = "grep -r pattern .";
        assert_eq!(strip_wrappers(plain), plain);
        assert_eq!(cmd_base(plain), "grep");
        // Re-applying either helper to its own output is a no-op.
        assert_eq!(strip_wrappers(strip_wrappers(plain)), plain);
        assert_eq!(cmd_base(cmd_base(plain)), cmd_base(plain));
    }
}
