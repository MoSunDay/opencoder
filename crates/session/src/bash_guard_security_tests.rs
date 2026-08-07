use super::*;

#[test]
fn shell_interpreters_with_c_flag_blocked() {
    assert!(matches!(
        classify("bash -c 'rm -rf /tmp/x'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("sh -c 'echo malicious'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("zsh -c 'touch /tmp/pwned'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("dash -c 'whoami'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("sudo bash -c 'rm x'"),
        BashVerdict::WriteBlocked(_)
    ));
    // Bare interpreter (no -c/-s) is allowed
    assert_eq!(classify("bash --version"), BashVerdict::ReadOnly);
    assert_eq!(classify("sh"), BashVerdict::ReadOnly);
}

#[test]
fn shell_interpreters_with_s_flag_blocked() {
    assert!(matches!(classify("bash -s"), BashVerdict::WriteBlocked(_)));
}

#[test]
fn script_interpreters_with_exec_flag_blocked() {
    assert!(matches!(
        classify("python3 -c 'import os; os.remove(\"x\")'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("python -c 'print(1)'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("node -e 'require(\"fs\").unlinkSync(\"x\")'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("ruby -e 'puts 1'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("perl -e 'system(\"rm x\")'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("perl -pe 's/a/b/'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("php -r 'echo 1;'"),
        BashVerdict::WriteBlocked(_)
    ));
    // Bare interpreter (no -c/-e/-r) is allowed
    assert_eq!(classify("python3 --version"), BashVerdict::ReadOnly);
    assert_eq!(classify("node --version"), BashVerdict::ReadOnly);
}

#[test]
fn xargs_always_blocked() {
    assert!(matches!(
        classify("echo x | xargs rm"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("find . | xargs rm"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("xargs echo"),
        BashVerdict::WriteBlocked(_)
    ));
}

#[test]
fn find_with_exec_or_delete_blocked() {
    assert!(matches!(
        classify("find . -exec rm {} \\;"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("find /tmp -delete"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("find . -execdir chmod +x {} +"),
        BashVerdict::WriteBlocked(_)
    ));
    // Read-only find is allowed
    assert_eq!(classify("find . -name '*.rs'"), BashVerdict::ReadOnly);
    assert_eq!(
        classify("find . -type f -name '*.go'"),
        BashVerdict::ReadOnly
    );
}

#[test]
fn install_and_truncate_blocked() {
    assert!(matches!(
        classify("install -m 755 script /usr/local/bin/"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("truncate -s 0 file.txt"),
        BashVerdict::WriteBlocked(_)
    ));
}

#[test]
fn interpreter_in_compound_command_blocked() {
    // Any mutating segment blocks the whole command.
    assert!(matches!(
        classify("ls && python3 -c 'import os; os.remove(\"x\")'"),
        BashVerdict::WriteBlocked(_)
    ));
    assert!(matches!(
        classify("cat file | bash -c 'read line; rm x'"),
        BashVerdict::WriteBlocked(_)
    ));
}
