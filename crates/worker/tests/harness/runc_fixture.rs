use serde_json::json;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Environment {
    pub fn new(root: &Path) -> Self {
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex/auth.json"), "fixture-login").unwrap();
        let agents = root.join("agents");
        std::fs::create_dir_all(agents.join("codex-runc")).unwrap();
        std::fs::create_dir_all(agents.join("prompts/probe/v1")).unwrap();
        std::fs::write(
            agents.join("codex-runc/meta.json"),
            json!({"name":"codex-runc","harness":"codex","current":{"prompt":"probe"}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            agents.join("prompts/probe/meta.json"),
            json!({"name":"probe","current":1,"history":[1]}).to_string(),
        )
        .unwrap();
        std::fs::write(
            agents.join("prompts/probe/v1/soul.md"),
            "Sandbox Codex probe",
        )
        .unwrap();
        let mut env = Self(vec![]);
        for (key, value) in [
            ("HOME", Some(home.display().to_string())),
            ("CODEX_HOME", None),
            ("OPENCODER_AGENTS_DIR", Some(agents.display().to_string())),
        ] {
            env.0.push((key, std::env::var_os(key)));
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        env
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in self.0.iter().rev() {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

pub fn runner() -> PathBuf {
    // Test binaries run from several layouts (`target/<profile>/deps`,
    // `target/<profile>/build/<pkg>/<hash>/out`, ...): the example binaries
    // live in the profile directory, a bounded number of ancestors up.
    let mut directory = std::env::current_exe().unwrap();
    for _ in 0..8 {
        let Some(parent) = directory.parent() else {
            break;
        };
        let binary = parent.join("examples/agent-step-runner");
        if binary.is_file() {
            return binary;
        }
        directory = parent.to_path_buf();
    }
    panic!("missing runner {}", directory.display());
}

/// The per-test runners directory (`dag.runners_dir`): this node's own
/// `dag-runner` and `agent-step-runner` beside the fake harness `codex` and
/// its startup script. Every sandbox binds the directory read-only at
/// `/run/opencoder/bin`, so the harness is reached there; every other
/// program a step runs comes from the host mapping and is not installed.
pub fn runners(root: &Path, runner: &Path) {
    std::fs::create_dir_all(root).unwrap();
    install(root, runner);
    install(root, &runner.parent().unwrap().join("dag-runner"));
    executable(root, "codex", include_str!("runc_codex.sh"));
    executable(
        root,
        "codex-start.sh",
        r#"set -eu
printf '%s\n' "$OPENCODER_HARNESS_CONTEXT" > "$OPENCODER_STEP_DIR/startup-context.json"
printf 'before\n' > "$OPENCODER_STEP_DIR/startup-state"
"$@"
printf 'after\n' >> "$OPENCODER_STEP_DIR/startup-state"
"#,
    );
}

/// Strip-copy one runner into the directory, keeping its name.
fn install(directory: &Path, source: &Path) {
    // Debug runners carry large symbol tables. Keep private copies small
    // without changing the executable code; the shared libraries they need
    // are the host's, bound read-only by the sandbox.
    assert!(Command::new("strip")
        .arg("--strip-debug")
        .arg("-o")
        .arg(directory.join(source.file_name().unwrap()))
        .arg(source)
        .status()
        .unwrap()
        .success());
}

/// Write a harness script the sandbox executes by its `/run/opencoder/bin`
/// name.
fn executable(directory: &Path, name: &str, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(&path, source).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
