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

/// Regression for bug B1: command wrappers (`env`, `nohup`, `timeout`, …) used
/// to mask the real command so plan-mode writes slipped through as ReadOnly.
/// `classify_segment` now strips wrappers before extracting the command name.
#[test]
fn wrapper_commands_dont_mask_writes() {
    // Each of these wraps a mutating command and must still be blocked.
    assert!(matches!(
        classify("env rm file"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(classify("nohup rm"), BashVerdict::WriteBlocked(_)));
    assert!(matches!(
        classify("timeout 10 rm -rf x"),
        BashVerdict::WriteBlocked(_)
    ));
    // Double sudo is still fully unwrapped.
    assert!(matches!(
        classify("sudo sudo rm"),
        BashVerdict::WriteBlocked(_)
    ));
    // Other wrappers over mutating commands.
    assert!(matches!(
        classify("nice rm file"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("command mv a b"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("ionice rm file"),
        BashVerdict::WriteBlocked(_)
    ));
    // Nested wrappers are fully unwrapped before classification.
    assert!(matches!(
        classify("env FOO=bar nohup rm file"),
        BashVerdict::WriteBlocked(_)
    ));
    // sudo + wrapper + mutating command.
    assert!(matches!(
        classify("sudo env rm file"),
        BashVerdict::WriteBlocked(_)
    ));
}

/// `env` with only variable assignments (no real write beneath it) stays
/// read-only — the wrapper stripping must not over-block.
#[test]
fn env_with_only_assignment_is_read_only() {
    assert_eq!(classify("env FOO=bar ls"), BashVerdict::ReadOnly);
    assert_eq!(classify("env VAR=1 git status"), BashVerdict::ReadOnly);
    assert_eq!(
        classify("env A=1 B=2 cat file"),
        BashVerdict::ReadOnly
    );
}

/// `exec`/`eval`/`source` keep their dedicated verdict even though `exec` is
/// also a wrapper that `strip_wrappers` would otherwise peel away.
#[test]
fn exec_eval_source_still_blocked_directly() {
    assert!(matches!(classify("exec ls"), BashVerdict::WriteBlocked(_)));
    assert!(matches!(
        classify("eval 'rm x'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("source script.sh"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(classify("sudo exec ls"), BashVerdict::WriteBlocked(_)));
    // But `env exec rm` unwraps to the mutating command beneath.
    assert!(matches!(
        classify("env exec rm file"),
        BashVerdict::WriteBlocked(_)
    ));
}

// ---------------------------------------------------------------------------
// Shared command-parsing helpers (extracted from tools::ssh_pty)
// ---------------------------------------------------------------------------

#[test]
fn strip_leading_sudo_peels_privilege_escalators() {
    assert_eq!(strip_leading_sudo("sudo rm -rf /"), "rm -rf /");
    assert_eq!(strip_leading_sudo("doas vim"), "vim");
    assert_eq!(strip_leading_sudo("sudo doas vim"), "vim");
    assert_eq!(strip_leading_sudo("ls"), "ls");
}

#[test]
fn cmd_base_extracts_binary_name() {
    assert_eq!(cmd_base("/usr/bin/vim"), "vim");
    assert_eq!(cmd_base("ls -la"), "ls");
    assert_eq!(cmd_base("python3"), "python3");
}

#[test]
fn strip_wrappers_unwraps_delegating_prefixes() {
    assert_eq!(strip_wrappers("env rm file"), "rm file");
    assert_eq!(strip_wrappers("env FOO=bar ls"), "ls");
    assert_eq!(strip_wrappers("nohup rm"), "rm");
    assert_eq!(strip_wrappers("timeout 5 rm -rf x"), "rm -rf x");
    assert_eq!(strip_wrappers("nice rm"), "rm");
    assert_eq!(strip_wrappers("command mv a b"), "mv a b");
    assert_eq!(strip_wrappers("sudo env rm"), "rm");
    assert_eq!(strip_wrappers("strace -f rm x"), "rm x");
    // Read-only command passes through unwrapped.
    assert_eq!(strip_wrappers("ls -la"), "ls -la");
}

