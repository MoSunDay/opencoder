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

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_commands_pass() {
        assert_eq!(classify("ls -la"), BashVerdict::ReadOnly);
        assert_eq!(classify("cat file.txt"), BashVerdict::ReadOnly);
        assert_eq!(classify("grep -r foo ."), BashVerdict::ReadOnly);
        assert_eq!(classify("find . -name '*.rs'"), BashVerdict::ReadOnly);
        assert_eq!(classify("git status"), BashVerdict::ReadOnly);
        assert_eq!(classify("git log --oneline"), BashVerdict::ReadOnly);
        assert_eq!(classify("git diff"), BashVerdict::ReadOnly);
        assert_eq!(classify("echo hello"), BashVerdict::ReadOnly);
        assert_eq!(classify("pwd"), BashVerdict::ReadOnly);
        assert_eq!(classify("head -n 10 file"), BashVerdict::ReadOnly);
        assert_eq!(classify("wc -l file"), BashVerdict::ReadOnly);
        assert_eq!(classify(""), BashVerdict::ReadOnly);
        assert_eq!(classify("true"), BashVerdict::ReadOnly);
    }

    #[test]
    fn file_write_redirects_blocked() {
        assert!(matches!(
            classify("echo x > file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("echo x >> file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cmd &> file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("echo x 2> file"),
            BashVerdict::WriteBlocked(_)
        ));
        // No-space variants still blocked.
        assert!(matches!(
            classify("echo x >file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cmd 2>>file"),
            BashVerdict::WriteBlocked(_)
        ));
        // /dev/null with trailing path chars is NOT /dev/null.
        assert!(matches!(
            classify("cmd > /dev/null/sneaky"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cmd 2>/dev/nullx"),
            BashVerdict::WriteBlocked(_)
        ));
        // A real file redirect mixed with /dev/null is still blocked.
        assert!(matches!(
            classify("echo x > file >/dev/null"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn devnull_and_fd_merge_redirects_allowed() {
        // Discarding output to /dev/null is read-only.
        assert_eq!(classify("cmd >/dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd > /dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd 2>/dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd &>/dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd 1>/dev/null"), BashVerdict::ReadOnly);
        // fd merges (dup2) don't write files.
        assert_eq!(classify("cmd 2>&1"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd 1>&2"), BashVerdict::ReadOnly);
        assert_eq!(classify("cmd >&1"), BashVerdict::ReadOnly);
        // In a pipeline — the pipe splits segments, both read-only.
        assert_eq!(classify("grep foo file 2>&1 | head"), BashVerdict::ReadOnly);
        // Multiple /dev/null redirects are fine.
        assert_eq!(
            classify("cmd >/dev/null 2>/dev/null"),
            BashVerdict::ReadOnly
        );
    }

    #[test]
    fn redirect_bypass_in_compound_blocked() {
        // /dev/null is allowed but the trailing rm segment is blocked.
        assert!(matches!(
            classify("cmd 2>&1; rm -rf x"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cmd >/dev/null && rm file"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn mutating_commands_blocked() {
        assert!(matches!(classify("rm -rf /"), BashVerdict::WriteBlocked(_)));
        assert!(matches!(classify("mv a b"), BashVerdict::WriteBlocked(_)));
        assert!(matches!(classify("cp a b"), BashVerdict::WriteBlocked(_)));
        assert!(matches!(
            classify("mkdir newdir"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("touch newfile"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("chmod +x script"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("kill -9 1234"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("dd if=/dev/zero of=file"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn git_writes_blocked() {
        assert!(matches!(classify("git push"), BashVerdict::WriteBlocked(_)));
        assert!(matches!(
            classify("git commit -m msg"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("git merge feature"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("git reset --hard"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("git checkout -- file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("git stash"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn git_reads_allowed() {
        assert_eq!(classify("git status"), BashVerdict::ReadOnly);
        assert_eq!(classify("git log --oneline -5"), BashVerdict::ReadOnly);
        assert_eq!(classify("git diff HEAD~1"), BashVerdict::ReadOnly);
        assert_eq!(classify("git branch"), BashVerdict::ReadOnly);
        assert_eq!(classify("git show HEAD"), BashVerdict::ReadOnly);
        assert_eq!(classify("git blame file.rs"), BashVerdict::ReadOnly);
    }

    #[test]
    fn package_managers_blocked() {
        assert!(matches!(
            classify("apt install foo"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("pip install requests"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("npm install express"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cargo install ripgrep"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("brew install htop"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn package_manager_reads_allowed() {
        assert_eq!(classify("pip list"), BashVerdict::ReadOnly);
        assert_eq!(classify("npm list"), BashVerdict::ReadOnly);
        assert_eq!(classify("cargo --version"), BashVerdict::ReadOnly);
        assert_eq!(classify("apt list --installed"), BashVerdict::ReadOnly);
    }

    #[test]
    fn inplace_editors_blocked() {
        assert!(matches!(
            classify("sed -i 's/a/b/' file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("perl -pi -e 's/a/b/' file"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn compound_commands_checked_per_segment() {
        // Read-only compound
        assert_eq!(classify("ls && cat file"), BashVerdict::ReadOnly);
        assert_eq!(classify("echo a; echo b"), BashVerdict::ReadOnly);
        assert_eq!(classify("git log | head -5"), BashVerdict::ReadOnly);
        // Any mutating segment blocks the whole command
        assert!(matches!(
            classify("ls && rm file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("echo a; git push"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("cat file | tee copy"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn sudo_prefix_checked() {
        assert!(matches!(
            classify("sudo rm file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("sudo git push"),
            BashVerdict::WriteBlocked(_)
        ));
        assert_eq!(classify("sudo ls"), BashVerdict::ReadOnly);
    }

    #[test]
    fn blocked_reason_is_descriptive() {
        match classify("rm -rf /tmp") {
            BashVerdict::WriteBlocked(reason) => {
                assert!(
                    reason.contains("rm"),
                    "reason should mention the command: {reason}"
                );
            }
            BashVerdict::ReadOnly => panic!("rm should be blocked"),
        }
    }

    #[test]
    fn fd_merge_before_shell_metachars_allowed() {
        // Regression: `2>&1` immediately followed by `)`/`}`/`]` used to be
        // misread as target `&1)` and blocked. All read-only.
        assert_eq!(classify("(echo hi 2>&1)"), BashVerdict::ReadOnly);
        assert_eq!(classify("{ ls 2>&1; }"), BashVerdict::ReadOnly);
        assert_eq!(classify("(make 2>&1)"), BashVerdict::ReadOnly);
        assert_eq!(classify("(echo hi 1>&2)"), BashVerdict::ReadOnly);
        assert_eq!(classify("(echo hi >&2)"), BashVerdict::ReadOnly);
        assert_eq!(classify("make 2>&1)"), BashVerdict::ReadOnly);
        // /dev/null before a closing metachar is read-only too.
        assert_eq!(classify("(cmd >/dev/null)"), BashVerdict::ReadOnly);
        assert_eq!(classify("{ cmd 2>/dev/null; }"), BashVerdict::ReadOnly);
        assert_eq!(classify("[ cmd 2>/dev/null ]"), BashVerdict::ReadOnly);
    }

    #[test]
    fn real_file_redirect_before_metachar_still_blocked() {
        // Genuine file writes right before `)`/`}` must still be blocked —
        // the boundary fix must not over-loosen.
        assert!(matches!(
            classify("(echo x > file)"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("{ echo x 2> err.log; }"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("(echo x >> log)"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("(echo x &> all.out)"),
            BashVerdict::WriteBlocked(_)
        ));
    }

    #[test]
    fn subshell_and_brace_group_read_only() {
        assert_eq!(classify("(echo hi)"), BashVerdict::ReadOnly);
        assert_eq!(classify("{ ls -la; }"), BashVerdict::ReadOnly);
        assert_eq!(classify("(git status)"), BashVerdict::ReadOnly);
    }

    #[test]
    fn tee_to_devnull_or_bare_allowed() {
        // tee writing to /dev/null (or nowhere) is read-only.
        assert_eq!(classify("echo x | tee /dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("echo x | tee"), BashVerdict::ReadOnly);
        assert_eq!(classify("tee -a /dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("echo x | tee -a /dev/null"), BashVerdict::ReadOnly);
        assert_eq!(classify("sudo tee /dev/null"), BashVerdict::ReadOnly);
    }

    #[test]
    fn tee_to_real_file_blocked() {
        // tee writing to any non-/dev/null path is a real write.
        assert!(matches!(
            classify("echo x | tee file"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("tee -a f.log"),
            BashVerdict::WriteBlocked(_)
        ));
        assert!(matches!(
            classify("echo x | tee a b"),
            BashVerdict::WriteBlocked(_)
        ));
        // One /dev/null plus one real file is still a write.
        assert!(matches!(
            classify("echo x | tee /dev/null file"),
            BashVerdict::WriteBlocked(_)
        ));
    }
}
