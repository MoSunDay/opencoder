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
pub fn classify(command: &str) -> BashVerdict {
    use opencoder_shellguard::Decision;
    let verdict = opencoder_shellguard::classify(command);
    match verdict.decision {
        Decision::Allow => BashVerdict::ReadOnly,
        Decision::Ask | Decision::Deny => BashVerdict::WriteBlocked(verdict.reason),
    }
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
    fn helpers_are_idempotent_on_plain_commands() {
        let plain = "grep -r pattern .";
        assert_eq!(strip_wrappers(plain), plain);
        assert_eq!(cmd_base(plain), "grep");
        // Re-applying either helper to its own output is a no-op.
        assert_eq!(strip_wrappers(strip_wrappers(plain)), plain);
        assert_eq!(cmd_base(cmd_base(plain)), cmd_base(plain));
    }
}
