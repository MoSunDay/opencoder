//! Bash command write-detection for plan-mode enforcement.
//!
//! In plan mode the agent must not modify the system. Rather than removing
//! `bash` entirely (it's useful for `ls`, `cat`, `grep`, `find`), we classify
//! each command as read-only or potentially-mutating and block the latter.
//!
//! The classifier is heuristic: it parses the command string for known
//! write patterns (redirects, mutating commands, package managers, git writes,
//! in-place editors). False positives are acceptable (over-blocking in plan
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
    "tee",
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

    // Check for redirect operators anywhere in the command (>, >>, &>).
    if has_redirect(trimmed) {
        return BashVerdict::WriteBlocked("redirect operator (>/>>)".into());
    }

    // Split into segments by &&, ;, |, and check each.
    for segment in split_segments(trimmed) {
        if let Some(reason) = classify_segment(&segment) {
            return BashVerdict::WriteBlocked(reason);
        }
    }

    BashVerdict::ReadOnly
}

/// Detect redirect operators: `>` (but not `->` or `2>` in a comparison),
/// `>>`, `&>`, `>&`.
fn has_redirect(cmd: &str) -> bool {
    let chars: Vec<char> = cmd.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if c == '>' {
            // Skip "2>" etc. (fd redirect) — still a redirect!
            return true;
        }
        // Check for &> (redirect stdout+stderr)
        if c == '&' && i + 1 < chars.len() && chars[i + 1] == '>' {
            return true;
        }
    }
    false
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
    fn redirects_blocked() {
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
}
