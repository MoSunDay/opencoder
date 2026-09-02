//! Pipeline-level tests: parser failure handling, compound commands, nesting
//! depth, and dispatcher behaviour over the ported rippy core.

use crate::test_support::{analyze_with, MockLookup};
use crate::verdict::Decision;

fn lk(pairs: &[(&str, &str)]) -> MockLookup {
    let mut l = MockLookup::new();
    for (name, val) in pairs {
        l = l.with(name, val);
    }
    l
}

#[test]
fn unparseable_command_falls_back_to_ask() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "ls &&&");
    assert_eq!(v.decision, Decision::Ask, "got {v:?}");
}

#[test]
fn pipe_allows_when_both_sides_allow() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "ls | grep foo");
    assert_eq!(v.decision, Decision::Allow, "got {v:?}");
    assert!(v.reason.contains("grep"), "reason: {}", v.reason);
}

#[test]
fn pipe_blocks_when_one_side_blocks() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "ls | rm -rf /var/x");
    assert_eq!(v.decision, Decision::Ask, "got {v:?}");
}

#[test]
fn and_allows_when_both_sides_allow() {
    let v = analyze_with(
        "/tmp".into(),
        lk(&[("FOO", "bar")]),
        "echo $FOO && cat /tmp/a.txt",
    );
    assert_eq!(v.decision, Decision::Allow, "got {v:?}");
}

#[test]
fn and_blocks_when_one_side_blocks() {
    // The cwd is a project dir (never released), so `rm x` targets it.
    let v = analyze_with(
        "/home/user/project".into(),
        MockLookup::new(),
        "cat /tmp/a.txt && rm x",
    );
    assert_eq!(v.decision, Decision::Ask, "got {v:?}");
    assert!(v.reason.contains("rm"), "reason: {}", v.reason);
}

#[test]
fn semicolon_sequence_allows_and_blocks() {
    let ok = analyze_with("/tmp".into(), MockLookup::new(), "ls; echo done");
    assert_eq!(ok.decision, Decision::Allow, "got {ok:?}");
    let bad = analyze_with("/tmp".into(), MockLookup::new(), "ls; rm -rf /var/x");
    assert_eq!(bad.decision, Decision::Ask, "got {bad:?}");
}

#[test]
fn command_substitution_is_fail_closed() {
    // Any command substitution asks: executing the substitution cannot be
    // gated statically, so even a benign `$(echo ...)` fails closed.
    let benign = analyze_with(
        "/home/user/project".into(),
        lk(&[("X", "hello")]),
        "echo $(echo $X) > /tmp/a.log",
    );
    assert_eq!(benign.decision, Decision::Ask, "got {benign:?}");
    assert!(
        benign.reason.contains("command substitution"),
        "reason: {}",
        benign.reason
    );
    let malicious = analyze_with(
        "/home/user/project".into(),
        MockLookup::new(),
        "echo $(rm /etc/x) > /tmp/a",
    );
    assert_eq!(malicious.decision, Decision::Ask, "got {malicious:?}");
}

#[test]
fn process_substitution_is_asked() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "cat <(ls)");
    assert_eq!(v.decision, Decision::Ask, "got {v:?}");
    assert!(
        v.reason.contains("process substitution"),
        "reason: {}",
        v.reason
    );
}

#[test]
fn nesting_overflow_asks() {
    // MAX_SUBSTITUTIONS is 128; one more `$(` must fail closed to Ask.
    let depth = 130;
    let cmd = format!("echo {}x{}", "$(".repeat(depth), ")".repeat(depth));
    let v = analyze_with("/tmp".into(), MockLookup::new(), &cmd);
    assert_eq!(v.decision, Decision::Ask, "got {v:?}");
    assert!(v.reason.contains("too complex"), "reason: {}", v.reason);
}

#[test]
fn allowlisted_commands_allow() {
    for cmd in ["ls", "cat /tmp/a.txt", "echo hi"] {
        let v = analyze_with("/tmp".into(), MockLookup::new(), cmd);
        assert_eq!(v.decision, Decision::Allow, "{cmd} -> {v:?}");
    }
}

#[test]
fn redirect_to_devnull_is_reported_in_reason() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "ls > /dev/null");
    assert_eq!(v.decision, Decision::Allow, "got {v:?}");
    assert!(
        v.reason.contains("redirect to /dev/null"),
        "reason: {}",
        v.reason
    );
}

#[test]
fn stderr_to_stdout_allows() {
    let v = analyze_with("/tmp".into(), MockLookup::new(), "ls 2>&1");
    assert_eq!(v.decision, Decision::Allow, "got {v:?}");
}
