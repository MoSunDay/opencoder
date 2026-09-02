//! Ported from rippy (MIT) https://github.com/mpecan/rippy (allowlist data trimmed to the sandbox policy).

use std::collections::HashSet;
use std::sync::LazyLock;

/// Commands known to be safe (read-only, no side effects).
/// Ported from Dippy's `SIMPLE_SAFE` frozenset.
static SIMPLE_SAFE: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        // File viewing
        "cat",
        "head",
        "tail",
        "less",
        "more",
        "bat",
        "hexdump",
        "strings",
        "xxd",
        "od",
        // Compressed file viewing
        "zcat",
        "bzcat",
        "xzcat",
        "zstdcat",
        // Binary analysis
        "nm",
        "objdump",
        "readelf",
        "ldd",
        "otool",
        "size",
        "file",
        // Directory listing
        "ls",
        "tree",
        "exa",
        "eza",
        "lsd",
        // File info
        "stat",
        "wc",
        "du",
        "df",
        // Text processing (read-only)
        "grep",
        "rg",
        "ag",
        "diff",
        "cut",
        "tr",
        // sort has a dedicated handler (handles -o output flag)
        "uniq",
        "paste",
        "join",
        "comm",
        "fold",
        "fmt",
        "nl",
        "column",
        "expand",
        "unexpand",
        "rev",
        "tac",
        "shuf",
        // Encoding/hashing
        "base64",
        "base32",
        "md5sum",
        "sha1sum",
        "sha256sum",
        "sha512sum",
        "cksum",
        "sum",
        // Search (find, fd, env, sort, yq have dedicated handlers — not in this list)
        "locate",
        "which",
        "whereis",
        "type",
        "whence",
        // System info
        "whoami",
        "hostname",
        "uname",
        "id",
        "groups",
        "uptime",
        "pwd",
        "date",
        // env has a dedicated handler (can delegate inner commands)
        "printenv",
        "locale",
        // Process info
        "ps",
        "top",
        "htop",
        "lsof",
        "vmstat",
        "iostat",
        "free",
        "pgrep",
        // Network info (read-only)
        "ping",
        "dig",
        "nslookup",
        "traceroute",
        "tracepath",
        "netstat",
        "ss",
        // ifconfig and ip have dedicated handlers
        "host",
        "getent",
        // Help/docs
        "man",
        "info",
        "whatis",
        "apropos",
        "tldr",
        "help",
        // Shell builtins (safe)
        "echo",
        "printf",
        "true",
        "false",
        "test",
        "[",
        ":",
        // Path manipulation
        "basename",
        "dirname",
        "realpath",
        "readlink",
        // Math
        "bc",
        "expr",
        "seq",
        // Misc read-only
        "tty",
        "stty",
        "tput",
        "yes",
        "sleep",
        // Version/capabilities
        "nproc",
        "getconf",
        "arch",
        "lsb_release",
        // Modern CLI tools
        "jq",
        // yq has a dedicated handler (handles -i inplace)
        "fzf",
        "tokei",
        "cloc",
        "scc",
        "hyperfine",
        // Encoding
        "iconv",
        // dos2unix/unix2dos have a dedicated handler (rewrite the named file in place by default)
        // Disk/fs info
        "mount",
        "findmnt",
        "lsblk",
        "blkid",
        // dmesg has a dedicated handler (clear flags)
    ])
});

/// `SIMPLE_SAFE` commands whose behavior *can* depend dangerously on an
/// argument value, so they must NOT be auto-allowed when an argument is a
/// set-but-unknown (attacker-influenceable) value such as a loop variable or a
/// glob match:
///
/// - pagers that can spawn a subshell (`!cmd`, `v`) or run an input
///   preprocessor (`LESSOPEN`): `less`, `more`, `man`, `info`
/// - interactive finders that execute a preview/bind command from an argument:
///   `fzf`
/// - commands that change system/terminal state from their argument: `mount`,
///   `stty`
///
/// They remain safe with *literal* arguments (still resolved and re-analyzed via
/// the normal path), but the dynamic-argument relaxation excludes them.
static DYNAMIC_ARG_UNSAFE: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| HashSet::from(["less", "more", "man", "info", "fzf", "mount", "stty"]));

/// Commands that wrap other commands — analyze the inner command instead.
static WRAPPER_COMMANDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "time", "timeout", "nice", "strace", "ltrace", "nohup", "command", "builtin",
    ])
});

/// Help/version flags the analyzer honors when one of them is a command's sole
/// argument — the whole `AllowReason::HelpFlag` surface.
///
/// Deliberately excludes `-h`/`-V`: commands overload them (`docker -h` is
/// `--hostname`), so a lone short flag keeps asking.
pub(crate) const SOLE_HELP_FLAGS: &[&str] = &["--help", "--version"];

/// Check if a command is in the simple-safe allowlist.
#[must_use]
pub(crate) fn is_simple_safe(cmd: &str) -> bool {
    SIMPLE_SAFE.contains(cmd)
}

/// Check if a command is a wrapper (should analyze inner command).
#[must_use]
pub(crate) fn is_wrapper(cmd: &str) -> bool {
    WRAPPER_COMMANDS.contains(cmd)
}

/// `timeout` flags that consume the following word as their value.
const TIMEOUT_VALUE_FLAGS: &[&str] = &["-k", "--kill-after", "-s", "--signal"];

/// `timeout` flags that stand alone.
const TIMEOUT_FLAGS: &[&str] = &["--preserve-status", "--foreground", "-v", "--verbose"];

/// The argv a wrapper actually executes, with the wrapper's own options removed.
///
/// Only `timeout` and `nice` put options in front of the command; every other
/// wrapper is passed through untouched on purpose. Both fall back to the whole
/// argv when the grammar does not match, which keeps the stray word as the
/// command name and so Asks. See docs/security-invariants.md#wrapper-redirects.
#[must_use]
pub(crate) fn wrapper_inner_args<'a>(cmd: &str, args: &'a [String]) -> &'a [String] {
    match cmd {
        "timeout" => timeout_inner_args(args).unwrap_or(args),
        "nice" => nice_inner_args(args).unwrap_or(args),
        _ => args,
    }
}

/// `nice [-n N | --adjustment=N | -N] COMMAND …`. Without this, `nice -n 10 ls`
/// read `-n` as the command and Asked on an ordinary safe invocation.
fn nice_inner_args(args: &[String]) -> Option<&[String]> {
    let mut i = 0;
    while let Some(arg) = args.get(i).map(String::as_str) {
        if arg == "-n" || arg == "--adjustment" {
            i += 2;
        } else if arg.starts_with("--adjustment=") || is_nice_adjustment(arg) {
            i += 1;
        } else {
            break;
        }
    }
    args.get(i..).filter(|rest| !rest.is_empty())
}

/// A bare adjustment such as `-10` or `-+5`, which `nice` accepts in place of
/// `-n 10`. A flag like `-n` is not one, so it still consumes its value.
fn is_nice_adjustment(arg: &str) -> bool {
    arg.strip_prefix('-').is_some_and(|rest| {
        let digits = rest.strip_prefix('+').unwrap_or(rest);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// `None` when the argv does not match GNU timeout's grammar, which keeps the
/// caller on the fail-closed path of treating the stray word as the command.
fn timeout_inner_args(args: &[String]) -> Option<&[String]> {
    let mut i = 0;
    while let Some(arg) = args.get(i).map(String::as_str) {
        if TIMEOUT_VALUE_FLAGS.contains(&arg) {
            i += 2;
        } else if TIMEOUT_FLAGS.contains(&arg) || is_timeout_joined_value(arg) {
            i += 1;
        } else {
            break;
        }
    }
    if is_timeout_duration(args.get(i)?) {
        args.get(i + 1..)
    } else {
        None
    }
}

fn is_timeout_joined_value(arg: &str) -> bool {
    arg.starts_with("--kill-after=")
        || arg.starts_with("--signal=")
        || (arg.len() > 2 && (arg.starts_with("-k") || arg.starts_with("-s")))
}

fn is_timeout_duration(arg: &str) -> bool {
    let body = arg.strip_suffix(['s', 'm', 'h', 'd']).unwrap_or(arg);
    body.starts_with(|c: char| c.is_ascii_digit())
        && body.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

/// Check if a command is safe to auto-allow even when one of its arguments is a
/// set-but-unknown (dynamic) value.
///
/// This is the `SIMPLE_SAFE` set minus the commands whose behavior can depend
/// dangerously on an argument value (`DYNAMIC_ARG_UNSAFE` — pagers, `fzf`,
/// `mount`, `stty`).
#[must_use]
pub(crate) fn is_dynamic_arg_safe(cmd: &str) -> bool {
    is_simple_safe(cmd) && !DYNAMIC_ARG_UNSAFE.contains(cmd)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_safe_commands() {
        for cmd in ["cat", "ls", "grep", "echo", "wc", "head"] {
            assert!(is_simple_safe(cmd), "{cmd} should be simple-safe");
        }
        for cmd in ["rm", "sudo", "curl", "sh", "mv"] {
            assert!(!is_simple_safe(cmd), "{cmd} must not be simple-safe");
        }
    }

    #[test]
    fn wrapper_detection_and_inner_argv() {
        for cmd in ["time", "timeout", "nice", "nohup", "command"] {
            assert!(is_wrapper(cmd), "{cmd} should be a wrapper");
        }
        assert!(!is_wrapper("ls"));
        assert_eq!(
            wrapper_inner_args("timeout", &["5".to_owned(), "ls".to_owned()]),
            ["ls".to_owned()].as_slice()
        );
    }

    #[test]
    fn dynamic_arg_relaxation_excludes_argument_sensitive_commands() {
        for cmd in ["cat", "ls", "echo", "wc"] {
            assert!(is_dynamic_arg_safe(cmd), "{cmd} should allow dynamic args");
        }
        for cmd in ["less", "more", "man", "fzf", "mount", "stty"] {
            assert!(
                !is_dynamic_arg_safe(cmd),
                "{cmd} must stay dynamic-arg-unsafe"
            );
        }
    }
}
