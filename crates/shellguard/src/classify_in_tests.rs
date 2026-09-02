//! `classify_in` cwd-injection contract (regression seam for the B2 fix):
//! the analyzer must resolve relative operands against the cwd it is *given*,
//! never against the process working directory. The bash tool executes in a
//! per-call `workdir`, so classification has to happen in that same directory
//! -- otherwise `touch f` is judged against the agent process's cwd while the
//! write actually lands in the executor's.
//!
//! Fixtures are real directories created without touching the process cwd.
//! The crate denies `unwrap`/`expect`/`panic` for all targets (tests
//! included), so creation is asserted, in the style of
//! `handlers::test_support`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::verdict::Decision;

/// Unique scratch directory under the literal `/tmp` release dir. NOT
/// `std::env::temp_dir()`: that honors `TMPDIR` and may live outside the
/// release set this policy hardcodes.
fn tmp_release_cwd(tag: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = Path::new("/tmp").join(format!("shellguard-classify-in-{n}-{tag}"));
    assert!(
        std::fs::create_dir_all(&dir).is_ok(),
        "failed to create {dir:?}"
    );
    dir
}

/// Unique scratch directory under $HOME: a plain directory that is never
/// inside the release set (the crate tree itself may sit under /tmp, which
/// the release scope covers wholesale).
fn plain_project_cwd(tag: &str) -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let home = std::env::var("HOME").unwrap_or_default();
    assert!(!home.is_empty(), "$HOME must be set for a plain fixture dir");
    let dir = Path::new(&home).join(format!("shellguard-plain-{n}-{tag}"));
    assert!(
        std::fs::create_dir_all(&dir).is_ok(),
        "failed to create {dir:?}"
    );
    dir
}

/// Best-effort fixture cleanup (mirrors `handlers::test_support::cleanup_dir`).
fn cleanup_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn relative_write_is_released_when_cwd_is_tmp() {
    let cwd = tmp_release_cwd("allow");
    let verdict = crate::classify_in("touch f", &cwd);
    assert_eq!(
        verdict.decision,
        Decision::Allow,
        "`touch f` must resolve under the released /tmp cwd, got {verdict:?}"
    );
    assert!(
        verdict.writes_state,
        "released mutation must retain typed write provenance"
    );
    cleanup_dir(&cwd);
}

#[test]
fn relative_write_is_blocked_when_cwd_is_not_tmp() {
    let cwd = plain_project_cwd("block");
    let verdict = crate::classify_in("touch f", &cwd);
    assert_ne!(
        verdict.decision,
        Decision::Allow,
        "`touch f` must not be released from a plain project cwd, got {verdict:?}"
    );
    assert!(!verdict.reason.is_empty(), "blocked verdict needs a reason");
    cleanup_dir(&cwd);
}

#[test]
fn cwd_alone_flips_the_verdict_for_the_same_command() {
    // Both legs classify the identical command string; only the injected cwd
    // differs. If either leg leaned on the process cwd this would collapse
    // to a single outcome.
    let released = tmp_release_cwd("flip-allow");
    let plain = plain_project_cwd("flip-block");
    let allowed = crate::classify_in("touch f", &released);
    let blocked = crate::classify_in("touch f", &plain);
    assert_eq!(allowed.decision, Decision::Allow, "got {allowed:?}");
    assert!(allowed.writes_state, "released leg must be marked as a write");
    assert_ne!(blocked.decision, Decision::Allow, "got {blocked:?}");
    cleanup_dir(&released);
    cleanup_dir(&plain);
}

#[test]
fn released_write_provenance_survives_compound_allow_ties() {
    let cwd = tmp_release_cwd("compound-write");
    let verdict = crate::classify_in("touch f && ls", &cwd);
    assert_eq!(verdict.decision, Decision::Allow, "got {verdict:?}");
    assert!(verdict.writes_state, "the trailing read must not mask the write");

    let read = crate::classify_in("ls >/dev/null", &cwd);
    assert_eq!(read.decision, Decision::Allow, "got {read:?}");
    assert!(!read.writes_state, "/dev/null does not persist state");
    cleanup_dir(&cwd);
}

/// #F9: the analyzer's cwd re-aim skips `cd` option flags exactly like the
/// `cd` handler (single implementation). `cd -P <released> && touch f` must
/// judge `touch f` against the released directory — taking `args.first()`
/// literally re-aimed at a `-P` subdirectory of the project instead.
#[test]
fn cd_option_flag_is_skipped_when_re_aiming_the_analysis_cwd() {
    let released = tmp_release_cwd("cd-flag-aim");
    let plain = plain_project_cwd("cd-flag-aim");
    let command = format!("cd -P {} && touch f", released.display());
    let verdict = crate::classify_in(&command, &plain);
    assert_eq!(
        verdict.decision,
        Decision::Allow,
        "`touch f` after `cd -P` into a released dir must be judged there, got {verdict:?}"
    );
    assert!(verdict.writes_state, "the released write must stay typed");
    // Without the flag skip the write was judged at <plain>/-P/f and Asked.
    cleanup_dir(&released);
    cleanup_dir(&plain);
}

/// #F9 counter-direction: an unrecognized `cd` flag Asks (handler) and the
/// list's re-aim is skipped entirely rather than guessed.
#[test]
fn cd_unknown_flag_still_asks_in_a_compound_list() {
    let released = tmp_release_cwd("cd-unknown-flag");
    let plain = plain_project_cwd("cd-unknown-flag");
    let command = format!("cd -Z {} && touch f", released.display());
    let verdict = crate::classify_in(&command, &plain);
    assert_ne!(
        verdict.decision,
        Decision::Allow,
        "an unknown cd flag must not release the trailing write, got {verdict:?}"
    );
    cleanup_dir(&released);
    cleanup_dir(&plain);
}
