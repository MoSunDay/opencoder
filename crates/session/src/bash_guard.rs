//! Bash command write-detection for sandbox-mode enforcement.
//!
//! In sandbox mode the agent must not modify the system. Rather than removing
//! `bash` entirely (it's useful for `ls`, `cat`, `grep`, `find`), we classify
//! each command as read-only or potentially-mutating and block the latter.
//!
//! The classifier is now a thin adapter over the [`opencoder-shellguard`]
//! crate, which is derived from [rippy](https://github.com/mpecan/rippy)
//! (MIT license, copyright the rippy authors). Sandbox policy: block all
//! risk-bearing writes; release `/dev/null` and `/tmp`; the cwd / project
//! directory is NOT released. `Allow` passes; `Ask`/`Deny` block.
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
    /// Command appears read-only; safe to execute in sandbox mode.
    ReadOnly,
    /// Command is blocked; carries a human-readable reason shown to the model.
    WriteBlocked(String),
}

/// Classify a bash command for sandbox-mode enforcement by delegating to
/// `opencoder_shellguard::classify` (derived from rippy, MIT). `Allow`
/// passes; `Ask`/`Deny` block, carrying the classifier's human-readable
/// reason (embedded verbatim in the tool error shown to the model).
///
/// Relative paths resolve against the *process* cwd. Production gating must
/// use [`classify_with_dir`] with the directory the command will run in:
/// the classification cwd must equal the execution cwd.
pub fn classify(command: &str) -> BashVerdict {
    use opencoder_shellguard::Decision;
    let verdict = opencoder_shellguard::classify(command);
    match verdict.decision {
        Decision::Allow => BashVerdict::ReadOnly,
        Decision::Ask | Decision::Deny => BashVerdict::WriteBlocked(verdict.reason),
    }
}

/// [`classify`] against an explicit working directory: relative operands
/// (`touch f`) resolve as if the shell were running in `cwd`. The caller
/// must pass the exact directory the command will execute in — for the bash
/// tool that is the per-call `workdir` input, defaulting to the session
/// working dir (see `tools::bash`).
pub fn classify_with_dir(command: &str, cwd: &std::path::Path) -> BashVerdict {
    use opencoder_shellguard::Decision;
    let verdict = opencoder_shellguard::classify_in(command, cwd);
    match verdict.decision {
        Decision::Allow => BashVerdict::ReadOnly,
        Decision::Ask | Decision::Deny => BashVerdict::WriteBlocked(verdict.reason),
    }
}

/// Tool names a sandbox session may execute, mirroring the `sandbox` agent's
/// `ToolFilter::Allow` list. `question` still has to clear the latent-skill
/// gate downstream; `bash` additionally passes the shellguard classifier.
const SANDBOX_ADMITTED: &[&str] = &["bash", "task", "question"];

/// Canonical model-facing denial for a sandbox interception. The wording is
/// the retry-suppression contract: name the mode, state the read-only
/// invariant, tell the model retries are futile, and point at the REAL
/// escape hatch (`/act`; there is no `/agent` command).
pub fn sandbox_denial(tool: &str, detail: &str) -> String {
    format!(
        "Blocked in sandbox mode: `{tool}` was not executed - sandbox mode is read-only and \
         nothing may be written ({detail}). Do not retry: every write attempt fails while \
         sandbox mode is active. To make changes, the user can switch to the act agent with `/act`."
    )
}

/// Sandbox execution gate for one tool call: `Some(denial)` refuses the call
/// with the model-visible [`sandbox_denial`], `None` lets it proceed.
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
/// - the session is sandbox but the tool is not admitted (a hallucinated or
///   remembered builtin like `edit`, or an unadvertised MCP tool): refuse so
///   a write can never slip through a tool the model was never shown;
/// - `bash` is admitted but the shellguard classifier flags the command as
///   mutating: refuse with the classifier's reason.
pub fn gate(
    kind: &opencoder_core::AgentKind,
    tool: &str,
    command: Option<&str>,
    workdir: &std::path::Path,
) -> Option<String> {
    if *kind != opencoder_core::AgentKind::Sandbox {
        return None;
    }
    if tool != "bash" && !SANDBOX_ADMITTED.contains(&tool) {
        return Some(sandbox_denial(tool, "tool is not available in sandbox mode"));
    }
    if tool == "bash" {
        if let BashVerdict::WriteBlocked(reason) = classify_with_dir(command.unwrap_or(""), workdir)
        {
            return Some(sandbox_denial("bash", &reason));
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
    fn tmp_release_passes() {
        assert_eq!(classify("echo x > /tmp/a.log"), BashVerdict::ReadOnly);
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
        assert!(matches!(classify("echo x > ./f"), BashVerdict::WriteBlocked(_)));
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
        let msg = super::sandbox_denial("edit", "tool is not available in sandbox mode");
        assert!(msg.contains("Blocked in sandbox mode"), "got: {msg}");
        assert!(msg.contains("read-only"), "got: {msg}");
        assert!(msg.contains("Do not retry"), "got: {msg}");
        assert!(msg.contains("`edit`"), "got: {msg}");
        // The escape hatch must be the REAL command: `/act`, never `/agent act`.
        assert!(msg.contains("`/act`"), "got: {msg}");
        assert!(!msg.contains("/agent act"), "got: {msg}");
    }

    #[test]
    fn admitted_set_matches_sandbox_agent_tool_filter() {
        let sandbox = opencoder_core::resolve_agent("sandbox").expect("sandbox agent");
        for name in super::SANDBOX_ADMITTED {
            assert!(
                sandbox.tools.allows(name),
                "gate admits {name} but the sandbox ToolFilter does not"
            );
        }
    }

    #[test]
    fn gate_passes_non_sandbox_kinds_through() {
        use opencoder_core::{resolve_agent, AgentKind};
        let anywhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["act", "explore"] {
            let agent = resolve_agent(name).unwrap();
            assert_eq!(
                super::gate(&agent.kind, "edit", Some("x"), anywhere),
                None,
                "{name} must not be gated"
            );
        }
        // Explicit non-sandbox kind is equally untouched.
        assert_eq!(
            super::gate(&AgentKind::Act, "bash", Some("rm -rf /"), anywhere),
            None
        );
    }

    #[test]
    fn gate_refuses_unadmitted_tool_in_sandbox() {
        use opencoder_core::resolve_agent;
        let kind = resolve_agent("sandbox").unwrap().kind;
        let anywhere = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for tool in ["edit", "bg", "mcp__fs__write"] {
            let denial = super::gate(&kind, tool, None, anywhere)
                .unwrap_or_else(|| panic!("{tool} must be refused"));
            assert!(denial.contains("Blocked in sandbox mode"), "got: {denial}");
        }
        // Admitted tools pass the first layer untouched.
        for tool in super::SANDBOX_ADMITTED {
            if *tool == "bash" {
                continue; // covered by the classifier tests below
            }
            assert_eq!(
                super::gate(&kind, tool, None, anywhere),
                None,
                "{tool} admitted"
            );
        }
    }

    #[test]
    fn gate_blocks_mutating_bash_in_sandbox() {
        use opencoder_core::resolve_agent;
        let kind = resolve_agent("sandbox").unwrap().kind;
        // A plain (non-/tmp) workdir: this crate's source tree.
        let plain = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(super::gate(&kind, "bash", Some("ls -la"), plain).is_none());
        let denial = super::gate(&kind, "bash", Some("rm -rf ./f"), plain).expect("blocked");
        assert!(denial.contains("Blocked in sandbox mode"), "got: {denial}");
    }

    #[test]
    fn gate_judges_bash_against_the_call_workdir_not_the_process_cwd() {
        use opencoder_core::resolve_agent;
        let kind = resolve_agent("sandbox").unwrap().kind;
        // The command is identical in both legs; only the effective workdir
        // differs. The default process cwd is this crate's source tree (never
        // released), so a gate still keyed on the process cwd would block the
        // /tmp leg.
        let tmp = std::path::Path::new("/tmp");
        assert_eq!(
            super::gate(&kind, "bash", Some("touch ./f"), tmp),
            None,
            "relative write under the /tmp workdir is released"
        );
        let plain = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let denial = super::gate(&kind, "bash", Some("touch ./f"), plain.path())
            .unwrap_or_else(|| panic!("same command must be blocked from a non-/tmp workdir"));
        assert!(denial.contains("Blocked in sandbox mode"), "got: {denial}");
    }

    #[test]
    fn classify_with_dir_resolves_relative_paths_against_the_given_cwd() {
        use super::classify_with_dir;
        // /tmp itself is in the release set: the relative target lands there.
        assert_eq!(
            classify_with_dir("touch f", std::path::Path::new("/tmp")),
            BashVerdict::ReadOnly
        );
        let plain = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
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
