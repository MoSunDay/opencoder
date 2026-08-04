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

