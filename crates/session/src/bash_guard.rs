//! Bash command write-detection for plan-mode enforcement.
//!
//! In plan mode the agent must not modify the system. Rather than removing
//! `bash` entirely (it's useful for `ls`, `cat`, `grep`, `find`), we classify
//! each command as read-only or potentially-mutating and block the latter.
//!
//! The classifier is heuristic: it parses the command string for known
//! write patterns (file-writing redirects, mutating commands, package managers,
//! git writes, in-place editors). False positives are acceptable (over-blocking in plan
//! mode is safe); false negatives are the risk we minimize by covering the
//! common patterns.

/// Verdict on whether a bash command may modify state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashVerdict {
    /// Command appears read-only; safe to execute in plan mode.
    ReadOnly,
    /// Command is blocked; carries a human-readable reason shown to the model.
    WriteBlocked(String),
}

/// Commands that unconditionally modify the filesystem or system state.
/// Matched as the first token (before any arguments). Case-sensitive: these
/// are conventionally lowercase.
const MUTATING_COMMANDS: &[&str] = &[
    "rm",
    "rmdir",
    "mv",
    "cp",
    "mkdir",
    "touch",
    "install",
    "truncate",
    "chmod",
    "chown",
    "ln",
    "dd",
    "mkfs",
    "mount",
    "umount",
    "fdisk",
    "parted",
    "kill",
    "pkill",
    "killall",
    "systemctl",
    "service",
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
];

/// Git subcommands that write state.
const GIT_WRITE_SUBS: &[&str] = &[
    "push",
    "commit",
    "merge",
    "rebase",
    "reset",
    "clean",
    "stash",
    "tag",
    "init",
    "clone",
    "fetch",
    "pull",
    "cherry-pick",
    "revert",
    "bisect",
    "worktree",
    "reflog",
    "update-ref",
    "symbolic-ref",
];

/// Package manager install/update commands.
const PACKAGE_MANAGERS: &[&str] = &[
    "apt", "apt-get", "yum", "dnf", "pacman", "zypper", "brew", "pip", "pip3", "pipx", "uv",
    "conda", "npm", "pnpm", "yarn", "bun", "cargo", "go", "gem", "composer",
];

/// Shell interpreters that execute arbitrary code when given `-c` or `-s`.
/// Matched as the first token (path-qualified names are handled via
/// `cmd_base`). Blocking is conditional on the `-c`/`-s` flag so a bare
/// `bash` or `sh` (e.g. checking `bash --version`) is not blocked.
const SHELL_INTERPRETERS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh", "fish"];

/// Script interpreters that execute inline code via `-c`/`-e`/`-r`.
/// Blocking is conditional on the flag so bare invocations are allowed.
const SCRIPT_INTERPRETERS: &[&str] = &[
    "python", "python3", "python2", "node", "ruby", "perl", "lua", "php", "php8",
];

/// Classify a bash command string.
///
/// Handles compound commands (`a && b`, `a; b`, `a | b`) by checking each
/// segment independently. If ANY segment is mutating, the whole command is
/// blocked.
pub fn classify(command: &str) -> BashVerdict {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return BashVerdict::ReadOnly;
    }

    // Check for file-writing redirect operators anywhere in the command.
    // Read-only redirects (/dev/null, fd merges like 2>&1) are allowed; only
    // redirects that write to a real file are blocked.
    if let Some(reason) = has_unsafe_redirect(trimmed) {
        return BashVerdict::WriteBlocked(reason);
    }

    // Split into segments by &&, ;, |, and check each.
    for segment in split_segments(trimmed) {
        if let Some(reason) = classify_segment(&segment) {
            return BashVerdict::WriteBlocked(reason);
        }
    }

    BashVerdict::ReadOnly
}

/// Detect *unsafe* redirect operators — those that write to a real file.
///
/// Read-only redirects are allowed: discarding output to `/dev/null` and
/// merging file descriptors (`2>&1`, `1>&2`) don't modify the filesystem.
/// File-writing redirects (`> file`, `>> file`, `2> file`) are blocked.
///
/// Scans the entire command string (before compound-command splitting) so a
/// dangerous redirect in any segment is caught.
fn has_unsafe_redirect(cmd: &str) -> Option<String> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if let Some(op_len) = match_redirect_op(&chars, i) {
            let target_start = i + op_len;
            let (ts, te) = read_redirect_target(&chars, target_start);
            let target: String = chars[ts..te].iter().collect();
            if !is_safe_redirect_target(&target) {
                return Some("redirect operator (>/>>)".into());
            }
            i = te;
        } else {
            i += 1;
        }
    }
    None
}

/// Try to match a redirect operator at position `i`.
/// Returns the operator length (chars consumed) on success.
fn match_redirect_op(chars: &[char], i: usize) -> Option<usize> {
    let n = chars.len();
    let c = chars[i];
    // &> / &>> (redirect both stdout and stderr to a file)
    if c == '&' && i + 1 < n && chars[i + 1] == '>' {
        return Some(if i + 2 < n && chars[i + 2] == '>' {
            3
        } else {
            2
        });
    }
    // [12]>> / [12]> (fd-prefixed redirect)
    if (c == '1' || c == '2') && i + 1 < n && chars[i + 1] == '>' {
        return Some(if i + 2 < n && chars[i + 2] == '>' {
            3
        } else {
            2
        });
    }
    // >> / > (bare redirect)
    if c == '>' {
        return Some(if i + 1 < n && chars[i + 1] == '>' {
            2
        } else {
            1
        });
    }
    None
}

/// Read the target token following a redirect operator, starting at `start`.
/// Skips leading whitespace; reads until a separator (whitespace, `;`, `|`,
/// or `&&`). Returns `(token_start, token_end)`.
fn read_redirect_target(chars: &[char], start: usize) -> (usize, usize) {
    let n = chars.len();
    let mut i = start;
    while i < n && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    let ts = i;
    // fd-merge form (`&N`, e.g. `2>&1`): capture exactly the `&` plus the
    // following ASCII digits. A trailing shell metacharacter (`)`, `}`, ...)
    // must NOT be folded into the target — otherwise `(echo 2>&1)` is read as
    // the target `&1)` and misclassified as a write.
    if i < n && chars[i] == '&' {
        i += 1; // consume `&`
        while i < n && chars[i].is_ascii_digit() {
            i += 1;
        }
        return (ts, i);
    }
    // Path form: read until a shell delimiter. Besides whitespace and the
    // compound separators, also stop at shell grouping / quoting / comment
    // metacharacters so a redirect immediately before `)`, `}`, `]`, `#`
    // terminates cleanly (e.g. `>/dev/null)`, `2>file}`).
    while i < n {
        let c = chars[i];
        if c == ' '
            || c == '\t'
            || c == ';'
            || c == '|'
            || c == ')'
            || c == '}'
            || c == ']'
            || c == '#'
        {
            break;
        }
        if c == '&' && i + 1 < n && chars[i + 1] == '&' {
            break;
        }
        i += 1;
    }
    (ts, i)
}

/// Whether a redirect target is read-only (doesn't write a file).
fn is_safe_redirect_target(target: &str) -> bool {
    // fd merge (&N): duplicate to an existing file descriptor.
    if let Some(rest) = target.strip_prefix('&') {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    // /dev/null discards output — read-only.
    target == "/dev/null"
}

/// Split a command string into individual segments by shell separators
/// (`&&`, `||`, `;`, `|`). Each segment is trimmed.
fn split_segments(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Check for two-char operators
        if i + 1 < chars.len() {
            let pair = format!("{}{}", c, chars[i + 1]);
            if pair == "&&" || pair == "||" {
                if !current.trim().is_empty() {
                    segments.push(current.trim().to_string());
                }
                current.clear();
                i += 2;
                continue;
            }
        }
        if c == ';' || c == '|' {
            if !current.trim().is_empty() {
                segments.push(current.trim().to_string());
            }
            current.clear();
            i += 1;
            continue;
        }
        current.push(c);
        i += 1;
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }
    segments
}

/// Classify a single command segment (no separators).
fn classify_segment(segment: &str) -> Option<String> {
    // Check for "sudo <cmd>" — look at the second word.
    let words: Vec<&str> = segment.split_whitespace().collect();
    let cmd_words = if words.first() == Some(&"sudo") || words.first() == Some(&"doas") {
        &words[1..]
    } else {
        &words[..]
    };

    if cmd_words.is_empty() {
        return None;
    }

    let cmd_name = cmd_words[0];
    let cmd_base = cmd_name.split('/').next_back().unwrap_or(cmd_name);

    // Check mutating commands
    if MUTATING_COMMANDS.contains(&cmd_base) {
        return Some(format!("mutating command: {cmd_base}"));
    }

    // `tee` is conditionally mutating: it duplicates stdin to its file
    // arguments. Writing to `/dev/null` (or no file argument at all) is
    // read-only; any other path argument is a real write and is blocked.
    if cmd_base == "tee" {
        let writes_real_file = cmd_words[1..]
            .iter()
            .any(|w| !w.starts_with('-') && *w != "/dev/null");
        if writes_real_file {
            return Some("tee (writes to file)".into());
        }
        return None;
    }

    // Check git writes
    if cmd_base == "git" || cmd_base == "hub" {
        if let Some(sub) = cmd_words.get(1) {
            if GIT_WRITE_SUBS.contains(sub) {
                return Some(format!("git {sub}"));
            }
            // "git checkout --" discards changes
            if *sub == "checkout" && cmd_words.contains(&"--") {
                return Some("git checkout --".into());
            }
        }
    }

    // Check package managers (only install/update/remove actions)
    if PACKAGE_MANAGERS.contains(&cmd_base) {
        if let Some(sub) = cmd_words.get(1) {
            let sub_lower = sub.to_lowercase();
            if matches!(
                sub_lower.as_str(),
                "install" | "update" | "upgrade" | "remove" | "uninstall" | "add" | "create"
            ) {
                return Some(format!("{cmd_base} {sub}"));
            }
        }
        // cargo with no subcommand but --install or similar flags
        if cmd_base == "cargo" && cmd_words.iter().any(|w| w == &"install") {
            return Some("cargo install".into());
        }
    }

    // Check in-place editors: sed -i, awk -i inplace, perl -i
    if cmd_base == "sed" && cmd_words.iter().any(|w| w == &"-i" || w.starts_with("-i")) {
        return Some("sed -i (in-place edit)".into());
    }
    if cmd_base == "awk" && cmd_words.iter().any(|w| w == &"-i" || w == &"--inplace") {
        return Some("awk -i (in-place edit)".into());
    }
    if cmd_base == "perl"
        && cmd_words.iter().any(|w| {
            // perl's `-i` (in-place edit) may be combined with other short flags
            // in a single token, e.g. `-pi`, `-nip`. No other lowercase perl
            // short flag contains 'i', so detecting it within a combined group is
            // unambiguous. `-I` (include path) is uppercase and excluded.
            w.starts_with('-') && !w.starts_with("--") && w.chars().skip(1).any(|c| c == 'i')
        })
    {
        return Some("perl -i (in-place edit)".into());
    }

    // Check for `exec`, `eval`, `source` — can run arbitrary mutating commands
    if matches!(cmd_base, "exec" | "eval" | "source" | ".") {
        return Some(format!("indirect execution: {cmd_base}"));
    }

    // Shell interpreters with -c/-s: `bash -c 'rm file'` etc.
    if SHELL_INTERPRETERS.contains(&cmd_base)
        && cmd_words[1..]
            .iter()
            .any(|w| *w == "-c" || *w == "-s")
    {
        return Some(format!("indirect execution: {cmd_base} -c/-s"));
    }

    // Script interpreters with -c/-e/-r: `python3 -c 'import os; os.remove(...)'` etc.
    // For perl, `-e` may be combined into a short-flag group like `-pe`, so
    // also scan combined flags (same logic as the `-i` check above).
    if SCRIPT_INTERPRETERS.contains(&cmd_base) {
        let has_exec_flag = if cmd_base == "perl" {
            cmd_words[1..].iter().any(|w| {
                *w == "-c"
                    || *w == "-e"
                    || (w.starts_with('-')
                        && !w.starts_with("--")
                        && w.chars().skip(1).any(|c| c == 'e'))
            })
        } else {
            cmd_words[1..]
                .iter()
                .any(|w| *w == "-c" || *w == "-e" || *w == "-r")
        };
        if has_exec_flag {
            return Some(format!("indirect execution: {cmd_base} interpreter"));
        }
    }

    // `xargs` runs arbitrary commands — block unconditionally.
    if cmd_base == "xargs" {
        return Some("indirect execution: xargs".into());
    }

    // `find` with -exec/-execdir/-delete/-ok/-okdir can mutate state.
    if cmd_base == "find"
        && cmd_words
            .iter()
            .any(|w| matches!(*w, "-exec" | "-execdir" | "-delete" | "-ok" | "-okdir"))
    {
        return Some("indirect execution: find (-exec/-delete)".into());
    }

    None
}

#[cfg(test)]
#[path = "bash_guard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bash_guard_security_tests.rs"]
mod security_tests;
