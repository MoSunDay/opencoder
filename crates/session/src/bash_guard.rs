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

/// Extract inner command strings from command/process substitution syntax:
/// `$(...)`, backticks, `<(...)`, and `>(...)`.
///
/// Uses balanced-paren matching for `$(...)` and `<(...)`/`>(...)`.
/// Backticks use paired matching.
fn extract_command_substitutions(cmd: &str) -> Vec<String> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut results = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // $(...)
        if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '(' {
            if let Some(end) = find_matching_paren(&chars, i + 1) {
                let inner: String = chars[i + 2..end].iter().collect();
                if !inner.trim().is_empty() {
                    results.push(inner);
                }
                i = end + 1;
                continue;
            }
        }
        // <(...) or >(...)
        if i + 1 < chars.len()
            && (chars[i] == '<' || chars[i] == '>')
            && chars[i + 1] == '('
        {
            if let Some(end) = find_matching_paren(&chars, i + 1) {
                let inner: String = chars[i + 2..end].iter().collect();
                if !inner.trim().is_empty() {
                    results.push(inner);
                }
                i = end + 1;
                continue;
            }
        }
        // Backtick
        if chars[i] == '`' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '`') {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                if !inner.trim().is_empty() {
                    results.push(inner);
                }
                i = i + 1 + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    results
}

/// Find the index of the `)` that matches the `(` at position `start`.
fn find_matching_paren(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&'(') {
        return None;
    }
    let mut depth = 0;
    for (i, &c) in chars.iter().enumerate().skip(start) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Classify a bash command string.
///
/// Handles compound commands (`a && b`, `a; b`, `a | b`) by checking each
/// segment independently. If ANY segment is mutating, the whole command is
/// blocked. A segment that is the right-hand side of a pipe and invokes an
/// interpreter without a script file (`curl … | sh`) is blocked as well: the
/// interpreter executes its piped stdin.
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

    // Recursively classify commands nested inside command/process
    // substitution: `$(...)`, backticks, `<(...)`, `>(...)`. Without this,
    // `echo "$(rm file)"` bypasses plan-mode because `echo` itself is
    // read-only but the substitution runs `rm`.
    for inner in extract_command_substitutions(trimmed) {
        match classify(&inner) {
            BashVerdict::WriteBlocked(reason) => {
                return BashVerdict::WriteBlocked(format!(
                    "nested command substitution: {reason}"
                ));
            }
            BashVerdict::ReadOnly => {}
        }
    }

    // Split into segments by &&, ;, |, and check each.
    for segment in split_segments(trimmed) {
        if let Some(reason) = classify_segment(&segment.text) {
            return BashVerdict::WriteBlocked(reason);
        }
        // The right-hand side of a pipe reads upstream output on stdin: an
        // interpreter invoked there with no script-file argument (`curl … |
        // sh`, `cat x.py | python -`) executes that input.
        if segment.stdin_from_pipe {
            if let Some(reason) = pipe_fed_interpreter_reason(&segment.text) {
                return BashVerdict::WriteBlocked(reason);
            }
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

/// One split command segment, plus whether its stdin is fed by a pipe
/// (i.e. the segment is the right-hand side of a `|` or `|&`).
struct Segment {
    text: String,
    stdin_from_pipe: bool,
}

/// Split a command string into individual segments by shell separators
/// (`&&`, `||`, `;`, `|`, `&`, newline). Each segment is trimmed and
/// remembers whether it follows a single `|` (pipe right-hand side).
fn split_segments(cmd: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut stdin_from_pipe = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Check for two-char operators
        if i + 1 < chars.len() {
            let pair = format!("{}{}", c, chars[i + 1]);
            if pair == "&&" || pair == "||" {
                push_segment(&mut segments, &mut current, stdin_from_pipe);
                stdin_from_pipe = false;
                i += 2;
                continue;
            }
            // `|&` pipes both stdout and stderr — the tail still reads a pipe.
            if pair == "|&" {
                push_segment(&mut segments, &mut current, stdin_from_pipe);
                stdin_from_pipe = true;
                i += 2;
                continue;
            }
        }
        if c == ';' || c == '|' || c == '&' || c == '\n' {
            push_segment(&mut segments, &mut current, stdin_from_pipe);
            stdin_from_pipe = c == '|';
            i += 1;
            continue;
        }
        current.push(c);
        i += 1;
    }
    push_segment(&mut segments, &mut current, stdin_from_pipe);
    segments
}

/// Append a finished segment unless it is blank; always resets the
/// accumulator so the next segment starts empty.
fn push_segment(segments: &mut Vec<Segment>, current: &mut String, stdin_from_pipe: bool) {
    let text = current.trim().to_string();
    if !text.is_empty() {
        segments.push(Segment {
            text,
            stdin_from_pipe,
        });
    }
    current.clear();
}

/// Control-flow tokens that can lead a segment once the compound-command
/// split has removed the separators: `if c; then rm x; fi` yields the
/// segments `if c`, `then rm x`, `fi`. Classifying the bare token (`then`,
/// `do`, …) as the command name would let the wrapped write through, so these
/// are stripped first. `!` (pipeline negation) and leading `{` (brace group)
/// are included for the same reason. Closer tokens (`fi`, `done`, `esac`,
/// `}`) are segment-final and can never hide a command, so they are not
/// listed.
const CONTROL_LEADS: &[&str] = &[
    "if", "then", "elif", "else", "do", "while", "until", "for", "!", "{",
];

/// Strip leading control-flow syntax from a segment so classification sees
/// the actual command: `then rm x` → `rm x`, `do rm x` → `rm x`,
/// `{ rm x` → `rm x`, `case $v in a) rm x` → `rm x`. Returns a slice of
/// `segment`; no allocation.
fn strip_leading_control(segment: &str) -> &str {
    let mut rest = segment.trim_start();
    loop {
        let mut progressed = false;
        // A leading `(` (or `((`) opens a subshell, not a command name.
        let no_parens = rest.trim_start_matches('(');
        if no_parens.len() != rest.len() {
            rest = no_parens.trim_start();
            progressed = true;
        }
        let Some(tok) = rest.split_whitespace().next() else {
            return rest;
        };
        if CONTROL_LEADS.contains(&tok) {
            rest = rest[tok.len()..].trim_start();
            progressed = true;
        } else if tok == "case" {
            // `case subject in pattern) cmds` — drop the header so the first
            // case-body command is inspected (labels are stripped on the next
            // loop iteration).
            let after_case = rest[tok.len()..].trim_start();
            let Some(subject) = after_case.split_whitespace().next() else {
                return after_case;
            };
            let after_subject = after_case[subject.len()..].trim_start();
            rest = if after_subject.split_whitespace().next() == Some("in") {
                after_subject["in".len()..].trim_start()
            } else {
                after_subject
            };
            progressed = true;
        } else if is_case_pattern_label(tok) {
            rest = rest[tok.len()..].trim_start();
            progressed = true;
        }
        if !progressed {
            return rest;
        }
    }
}

/// A `case` pattern label at the start of a segment: a glob/alternation word
/// terminated by `)` — `a)`, `*)`, `linux|darwin)`. Tokens containing `(`
/// (subshells, `foo()` function definitions) are not labels.
fn is_case_pattern_label(tok: &str) -> bool {
    tok.ends_with(')') && !tok.contains('(')
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
        "strace" | "ltrace" | "perf" | "valgrind" => {
            strip_wrappers(skip_option_tokens(rest, &[]))
        }
        _ => stripped,
    }
}

/// Block interpreters that read code from stdin fed by a pipe:
/// `curl … | sh` or `cat x.py | python -`. Without a script-file argument the
/// interpreter executes its piped input, so read-only-looking upstream
/// commands become arbitrary code execution. An interpreter with an explicit
/// script-file argument (`python script.py`) keeps the existing (allowed)
/// policy — as does a bare `sh` that is not the right-hand side of a pipe.
fn pipe_fed_interpreter_reason(segment: &str) -> Option<String> {
    let stripped = strip_wrappers(segment);
    let base = cmd_base(stripped);
    if !SHELL_INTERPRETERS.contains(&base) && !SCRIPT_INTERPRETERS.contains(&base) {
        return None;
    }
    let words: Vec<&str> = stripped.split_whitespace().collect();
    // No script-file argument: everything after the interpreter (if anything)
    // is an option, including the `-` read-stdin convention (`python -`).
    let has_script_arg = words[1..].iter().any(|w| !is_option_token(w));
    if has_script_arg {
        return None;
    }
    Some(format!("indirect execution: {base} (piped stdin)"))
}

/// Classify a single command segment (no separators).
fn classify_segment(segment: &str) -> Option<String> {
    // Compound-command splitting leaves control-flow tokens at the start of a
    // segment (`if c; then rm x; fi` → `then rm x`, `{ rm x`, `a) rm x`).
    // Strip them first so the real command — not `then`/`do`/the case label —
    // is classified.
    let unled = strip_leading_control(segment);
    // `exec`/`eval`/`source`/`.` can run arbitrary mutating commands. Inspect
    // the leading token (after sudo/doas) *before* wrapper stripping: `exec` is
    // itself a wrapper that `strip_wrappers` would peel away, so checking it up
    // front preserves its dedicated verdict regardless of what follows.
    let sudo_stripped = strip_leading_sudo(unled);
    let first_base = cmd_base(sudo_stripped);
    if matches!(first_base, "exec" | "eval" | "source" | ".") {
        return Some(format!("indirect execution: {first_base}"));
    }

    // Strip wrapper commands (`env`, `nohup`, `timeout`, `nice`, `command`,
    // `strace`, `time`, `stdbuf`, `setsid`, …) and leading `sudo`/`doas` to
    // reveal the real command. Without this, plan-mode writes wrapped as
    // `env rm file`, `nohup rm`, or `timeout 5 rm -rf x` are misclassified as
    // read-only and bypass the guard.
    let stripped = strip_wrappers(unled);
    let cmd_words: Vec<&str> = stripped.split_whitespace().collect();
    if cmd_words.is_empty() {
        return None;
    }
    let cmd_base = cmd_base(stripped);

    // Post-wrapper indirect execution check: `env eval 'rm file'` survives
    // the pre-wrapper check (base was `env`, not `eval`) because `eval` is
    // not a wrapper command. After stripping `env`, we must re-check.
    if matches!(cmd_base, "eval" | "source" | ".") {
        return Some(format!("indirect execution via wrapper: {cmd_base}"));
    }

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
    if cmd_base == "sed"
        && cmd_words.iter().any(|w| {
            // `-i` may carry an attached backup suffix (`-i.bak`); the GNU
            // long form is `--in-place` / `--in-place=.bak`.
            w.starts_with("-i") || *w == "--in-place" || w.starts_with("--in-place=")
        })
    {
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

    // Shell interpreters with -c/-s: `bash -c 'rm file'` etc.
    if SHELL_INTERPRETERS.contains(&cmd_base)
        && cmd_words[1..].iter().any(|w| *w == "-c" || *w == "-s")
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

    // `find` with -exec/-execdir/-delete/-ok/-okdir can mutate state, as can
    // the file-writing print actions -fprint/-fprint0/-fprintf/-fls (unlike
    // -print/-printf, which only write to stdout).
    if cmd_base == "find"
        && cmd_words.iter().any(|w| {
            matches!(
                *w,
                "-exec"
                    | "-execdir"
                    | "-delete"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
                    | "-fls"
            )
        })
    {
        return Some("indirect execution: find (-exec/-delete/-fprint)".into());
    }

    None
}

#[cfg(test)]
#[path = "bash_guard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bash_guard_security_tests.rs"]
mod security_tests;

#[cfg(test)]
#[path = "bash_guard_bypass_regression.rs"]
mod bypass_regression_tests;
