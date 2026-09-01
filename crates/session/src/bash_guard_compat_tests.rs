//! Compatibility corpus for the bash_guard → shellguard swap.
//!
//! Table-driven port of the old hand-rolled classifier corpus (freezing the
//! behavior-compat surface of bash_guard as it stood just before the
//! shellguard swap): every
//! row is a command string → expected verdict, decided against the
//! rippy-derived `opencoder-shellguard` classifier plus the strict plan
//! adapter: persistent writes are blocked even under sandbox-released `/tmp`;
//! non-persistent `/dev/null`/fd redirects stay allowed; cwd/project writes
//! and command-substitution risk fail closed.
//!
//! Divergence classes used in the tags below:
//! - `KEEP` (untagged): old and new verdicts agree.
//! - `RELEASE`: historical shellguard release behavior; plan now consumes
//!   typed write provenance and blocks persistent `/tmp` mutations.
//! - `RETARGETED`: the danger was the STRUCTURE (wrapper/substitution/
//!   compound bypass) but the target happened to be `/tmp`; the target moved
//!   to a non-release path so the structural invariant stays provable.
//! - `OVER-BLOCK (safe)`: old ReadOnly, new WriteBlocked. Never hidden.
//! - `RELAXED (verified safe)`: old WriteBlocked, new ReadOnly, no write to
//!   any path outside the release set and no code execution possible — the
//!   new classifier judges the payload instead of the shape.
//!
//! Genuine holes (old WriteBlocked → new ReadOnly that writes real state
//! outside the release set) must never be parked here: the former
//! `compat_known_holes` in-place rows (GNU `--in-place` forms, clustered
//! `-pi`) were fixed in the shellguard handlers and now live in
//! `compat_in_place_edits_are_blocked` as enforced regression rows.

use super::{classify_with_dir, BashVerdict};

/// Expected verdict for a compat row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Command must pass sandbox classification.
    ReadOnly,
    /// Command must be blocked (with a non-empty reason).
    Blocked,
}

/// One compat row: command string plus the ported expectation.
struct Row(&'static str, Expect);

fn blocked(cmd: &'static str) -> Row {
    Row(cmd, Expect::Blocked)
}

fn readonly(cmd: &'static str) -> Row {
    Row(cmd, Expect::ReadOnly)
}

/// Drive every row through the adapter and fail loudly on any divergence.
/// Every `WriteBlocked` must carry a non-empty reason (it is shown to the
/// model), so that is asserted for all blocked rows.
/// Rows classify against a PLAIN (non-released) working directory: the
/// corpus pins classifier verdicts, and the process cwd must never decide
/// them (the crate tree itself may sit under the released /tmp).
fn run_rows(rows: &[Row]) {
    let plain = super::plain_dir();
    for Row(cmd, expect) in rows {
        match (expect, classify_with_dir(cmd, plain.path())) {
            (Expect::ReadOnly, BashVerdict::ReadOnly) => {}
            (Expect::Blocked, BashVerdict::WriteBlocked(reason)) => {
                assert!(
                    !reason.trim().is_empty(),
                    "blocked row `{cmd}` must carry a non-empty reason"
                );
            }
            (expect, verdict) => {
                panic!("compat divergence for `{cmd}`: expected {expect:?}, got {verdict:?}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read-only baseline (ported from `read_only_commands_pass`,
// `git_reads_allowed`, `package_manager_reads_allowed`, `subshell_and_brace_group_read_only`,
// `compound_commands_checked_per_segment` read-only halves).
// ---------------------------------------------------------------------------

#[test]
fn compat_read_only_rows_stay_allowed() {
    run_rows(&[
        readonly("ls -la"),
        readonly("cat file.txt"),
        readonly("grep -r foo ."),
        readonly("find . -name '*.rs'"),
        readonly("git status"),
        readonly("git log --oneline"),
        readonly("git diff"),
        readonly("git log --oneline -5"),
        readonly("git diff HEAD~1"),
        readonly("git branch"),
        readonly("git show HEAD"),
        readonly("git blame file.rs"),
        readonly("echo hello"),
        readonly("pwd"),
        readonly("head -n 10 file"),
        readonly("wc -l file"),
        readonly(""),
        readonly("true"),
        readonly("npm list"),
        readonly("cargo --version"),
        readonly("ls && cat file"),
        readonly("echo a; echo b"),
        readonly("git log | head -5"),
        readonly("(echo hi)"),
        readonly("{ ls -la; }"),
        readonly("(git status)"),
        // fd-merge rows (old `fd_merge_before_shell_metachars_allowed`):
        // known read-only commands, so the release set is observable.
        readonly("(echo hi 2>&1)"),
        readonly("{ ls 2>&1; }"),
        readonly("(echo hi 1>&2)"),
        readonly("(echo hi >&2)"),
        readonly("grep foo file 2>&1 | head"),
    ]);
}

// ---------------------------------------------------------------------------
// Redirects to the release set and to real files.
// Ported from `file_write_redirects_blocked`, `devnull_and_fd_merge_redirects_allowed`,
// `tee_to_devnull_or_bare_allowed`, `tee_to_real_file_blocked`,
// `real_file_redirect_before_metachar_still_blocked`.
// ---------------------------------------------------------------------------

#[test]
fn compat_redirect_rows() {
    run_rows(&[
        // Real-file redirects stay blocked (no-space variants included).
        blocked("echo x > file"),
        blocked("echo x >> file"),
        blocked("cmd &> file"), // OVER-BLOCK (safe): placeholder `cmd` is not a known command; fail-closed
        blocked("echo x 2> file"),
        blocked("echo x >file"),
        blocked("cmd 2>>file"), // OVER-BLOCK (safe): placeholder `cmd` is not a known command; fail-closed
        blocked("echo x > file >/dev/null"),
        blocked("{ echo x 2> err.log; }"),
        blocked("(echo x >> log)"),
        blocked("(echo x &> all.out)"),
        // /dev/null + fd-merge release (with known commands so the release
        // itself is what is exercised).
        readonly("ls >/dev/null"),
        readonly("ls > /dev/null"),
        readonly("ls 2>/dev/null"),
        readonly("ls &>/dev/null"),
        readonly("ls 1>/dev/null"),
        readonly("ls >/dev/null 2>/dev/null"),
        readonly("ls 2>&1"),
        readonly("ls 1>&2"),
        readonly("ls >&1"),
        readonly("(ls >/dev/null)"),
        readonly("{ ls 2>/dev/null; }"),
        // Old `cmd …` placeholder rows are covered by the over-block test.
        // tee: release target / bare-tee stay read-only, real files blocked.
        readonly("echo x | tee /dev/null"),
        readonly("echo x | tee"),
        readonly("tee -a /dev/null"),
        readonly("echo x | tee -a /dev/null"),
        blocked("echo x | tee file"),
        blocked("tee -a f.log"),
        blocked("echo x | tee a b"),
        blocked("echo x | tee /dev/null file"),
        // Mixed release/non-release compound: the non-release half blocks.
        blocked("cp /tmp/a ./b"),
        blocked("mv /etc/x /tmp/y"),
    ]);
}

#[test]
fn compat_devnull_boundary_rows() {
    run_rows(&[
        // Trailing path chars are NOT /dev/null (component boundary holds).
        blocked("ls 2>/dev/nullx"),
        blocked("echo x > f >/dev/null"),
        // Only the exact device redirect is non-persistent. A nested path is
        // represented as a released-scope write and strict plan blocks it.
        blocked("ls > /dev/null/sneaky"),
    ]);
}

#[test]
fn compat_tmp_release_flips() {
    // Shellguard may release /tmp for sandbox use, but plan mode rejects every
    // persistent write using the verdict's typed write provenance.
    run_rows(&[
        blocked("echo x > /tmp/a.log"),
        blocked("echo x >> /tmp/x"),
        blocked("rm -rf /tmp"),
        blocked("rm -rf /tmp/build"),
        blocked("touch /tmp/x"),
        blocked("echo x > /tmp/x"),
        blocked("mv /tmp/a /tmp/b"),
        blocked("cp /tmp/a /tmp/b"),
        blocked("cd /tmp && rm -rf /tmp/x"),
    ]);
}

// ---------------------------------------------------------------------------
// OVER-BLOCK (safe): old ReadOnly → new WriteBlocked. Every row is a genuine
// behavioral divergence and is kept visible on purpose. The dominant new
// posture is fail-closed on commands outside the known-handler set (an
// unknown command could do anything, so it asks).
// ---------------------------------------------------------------------------

#[test]
fn compat_over_blocks_unknown_command_fail_close() {
    run_rows(&[
        // Package-manager reads the old guard allowed; their binaries are not
        // in the new known-command set.
        blocked("pip list"), // OVER-BLOCK (safe): pip is unknown → fail-closed
        blocked("apt list --installed"), // OVER-BLOCK (safe): apt is unknown → fail-closed
        // Privilege escalators are unknown → everything under sudo/doas asks.
        blocked("sudo ls"), // OVER-BLOCK (safe): sudo is unknown → fail-closed
        // Interpreters.
        blocked("sh"), // OVER-BLOCK (safe): bare interactive shell reads stdin (code execution risk)
        // Placeholder commands (`cmd`) the old guard only pattern-matched for
        // redirects are now fail-closed; the /dev/null release rows above use
        // real commands so the release itself stays observable.
        blocked("cmd >/dev/null"), // OVER-BLOCK (safe): unknown command → fail-closed
        blocked("cmd > /dev/null"),
        blocked("cmd 2>/dev/null"),
        blocked("cmd &>/dev/null"),
        blocked("cmd 1>/dev/null"),
        blocked("cmd 2>&1"),
        blocked("cmd 1>&2"),
        blocked("cmd >&1"),
        blocked("cmd >/dev/null 2>/dev/null"),
        blocked("(cmd >/dev/null)"),
        blocked("{ cmd 2>/dev/null; }"),
        // `[ cmd 2>/dev/null ]` keeps the old ReadOnly verdict: `[` is a known
        // test-command and `cmd` is only its argument, so the /dev/null
        // redirect is all that is checked.
        readonly("[ cmd 2>/dev/null ]"),
        // Old parse-artifact rows (trailing `)` / unknown binary) now fail closed.
        blocked("make 2>&1)"), // OVER-BLOCK (safe): unrecognized construct → fail-closed
        blocked("(make 2>&1)"), // OVER-BLOCK (safe): make is unknown → fail-closed
        // Wrappers that are not in the new known-command set.
        blocked("stdbuf -o0 grep -q x file"), // OVER-BLOCK (safe): stdbuf is unknown → fail-closed
    ]);
}

#[test]
fn compat_over_blocks_fail_closed_semantics() {
    run_rows(&[
        // Any $()/`` ``/`<(...)` substitution asks (fail-closed), even when the
        // payload is a pure read like `date`.
        blocked("echo \"$(date)\""), // OVER-BLOCK (safe): command substitution asks unconditionally
        // Interpreter + explicit script file on a pipe: script execution is
        // never released under the new policy.
        blocked("cat data.txt | python script.py"), // OVER-BLOCK (safe): interpreter script execution is not released
    ]);
}

// ---------------------------------------------------------------------------
// Interpreters (ported from `shell_interpreters_with_c_flag_blocked`,
// `script_interpreters_with_exec_flag_blocked`, and the relaxed counterparts).
// ---------------------------------------------------------------------------

#[test]
fn compat_interpreter_rows() {
    run_rows(&[
        // Payload analysis: dangerous inline code stays blocked.
        blocked("python3 -c 'import os; os.remove(\"x\")'"),
        blocked("node -e 'require(\"fs\").unlinkSync(\"x\")'"),
        blocked("perl -e 'system(\"rm x\")'"),
        blocked("perl -pe 's/a/b/'"),
        blocked("php -r 'echo 1;'"), // blocked (unknown command) — over-block, harmless payload
        blocked("sudo bash -c 'rm x'"), // blocked (unknown command) — over-block, dangerous payload
        blocked("bash -s"),
        blocked("sh -c 'rm x'"),
        // RELAXED (verified safe): the classifier recurses into -c/-e payloads
        // and allows the ones that provably write nothing.
        readonly("bash -c 'echo hi'"), // RELAXED (verified safe): payload `echo hi` writes nothing
        readonly("sh -c 'echo malicious'"), // RELAXED (verified safe): payload writes nothing
        readonly("dash -c 'whoami'"),  // RELAXED (verified safe): payload writes nothing
        readonly("python -c 'print(1)'"), // RELAXED (verified safe): payload writes nothing
        readonly("ruby -e 'puts 1'"),  // RELAXED (verified safe): payload writes nothing
        blocked("ruby -e 'File.delete(\"x\")'"), // payload analysis catches the delete (sanity boundary)
        // Version/help flags stay allowed.
        readonly("bash --version"),
        readonly("python3 --version"),
        readonly("node --version"),
        // Both sandbox-released and ordinary write targets are blocked.
        blocked("bash -c 'rm -rf /tmp/x'"),
        blocked("bash -c 'rm -rf /var/x'"), // RETARGETED: was /tmp (now released); structural invariant must hold
        blocked("zsh -c 'touch /tmp/pwned'"),
        blocked("zsh -c 'touch /var/x'"), // RETARGETED: was /tmp (now released); structural invariant must hold
    ]);
}

// ---------------------------------------------------------------------------
// find / sed / xargs (ported from `xargs_always_blocked`,
// `find_with_exec_or_delete_blocked`, `find_file_writing_actions_are_blocked`,
// `sed_long_in_place_form_is_blocked`, `sed_readonly_counterparts_stay_allowed`).
// ---------------------------------------------------------------------------

#[test]
fn compat_find_sed_xargs_rows() {
    run_rows(&[
        blocked("echo x | xargs rm"),
        blocked("find . | xargs rm"),
        readonly("xargs echo"), // RELAXED (verified safe): xargs target `echo` is read-only (xargs rm still blocked)
        blocked("find . -exec rm {} \\;"),
        blocked("find . -execdir chmod +x {} +"),
        readonly("find . -type f -name '*.go'"),
        readonly("find . -printf '%p\\n'"),
        blocked("find /tmp -delete"),
        blocked("find /var/tmp/oc-compat-dead -delete"), // RETARGETED: was /tmp (now released); structural invariant must hold
        blocked("find . -fprint /tmp/out"),
        blocked("find . -fprint /var/out"), // RETARGETED: was /tmp (now released); structural invariant must hold
        blocked("find . -fprintf /tmp/out '%p\\n'"),
        blocked("find . -fprintf /var/out '%p\\n'"), // RETARGETED: was /tmp (now released); structural invariant must hold
        blocked("find . -fls /tmp/out"),
        blocked("find . -fls /var/out"), // RETARGETED: was /tmp (now released); structural invariant must hold
        blocked("find . -fprint0 /tmp/out"),
        blocked("find . -fprint0 /var/out"), // RETARGETED: was /tmp (now released); structural invariant must hold
        // sed short form is detected; read-only sed stays allowed.
        blocked("sed -i 's/a/b/' file"),
        blocked("sed -i.bak s/a/b/ f"),
        readonly("sed -n 1p file"),
        readonly("sed 's/a/b/' f"),
    ]);
}

// ---------------------------------------------------------------------------
// In-place edits — the formerly parked `compat_known_holes` audit rows, now
// enforced: the sed handler classifies the GNU long forms (`--in-place`,
// `--in-place=<suffix>`) alongside the short `-i`/`-i.bak` forms, and the
// perl/ruby handlers detect the in-place `-i` inside clustered short flags
// (`-pi`, `-ipe`, `-pi.bak`). Every row must block and cite the in-place edit.
// ---------------------------------------------------------------------------

#[test]
fn compat_in_place_edits_are_blocked() {
    for (cmd, why) in [
        ("sed --in-place s/a/b/ f", "GNU long form"),
        (
            "sed --in-place=.bak s/a/b/ f",
            "GNU long form with backup suffix",
        ),
        ("sed -i.bak s/a/b/ f", "short form with glued backup suffix"),
        (
            "perl -pi -e 's/a/b/' file",
            "clustered -pi hides -i from the -e harvest",
        ),
        (
            "ruby -pi -e 'puts 1' file",
            "clustered -pi hides -i from the -e analysis",
        ),
    ] {
        let plain = super::plain_dir();
        match classify_with_dir(cmd, plain.path()) {
            BashVerdict::WriteBlocked(reason) => assert!(
                reason.to_lowercase().contains("in-place"),
                "row `{cmd}` ({why}) must cite the in-place edit, got: {reason}"
            ),
            other => panic!(
                "compat divergence for `{cmd}` ({why}): expected WriteBlocked, got {other:?}"
            ),
        }
    }
}
