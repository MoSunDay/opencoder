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
    let target = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let binary = target.join("examples/agent-step-runner");
    let profile = target.file_name().unwrap().to_str().unwrap();
    let profile = if profile == "debug" { "dev" } else { profile };
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "opencoder-dag-runtime",
            "--example",
            "agent-step-runner",
        ])
        // The outer invocation's --target-dir overrides CARGO_TARGET_DIR.
        // Build next to this test executable so we cannot use a stale runner
        // from a different target directory or build profile.
        .arg("--target-dir")
        .arg(target.parent().unwrap())
        .args(["--profile", profile])
        // Container fixtures strip symbols, so avoid generating them first.
        .arg("--config")
        .arg(format!("profile.{profile}.debug=0"))
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .status()
        .unwrap();
    assert!(status.success(), "build container runner");
    assert!(binary.is_file(), "missing runner {}", binary.display());
    binary
}

pub fn rootfs(root: &Path, runner: &Path) {
    opencoder_dag_runtime::sandbox::oci::write_rootfs_template(root).unwrap();
    install(root, runner, "/usr/bin/agent-step-runner");
    for binary in ["/bin/sh", "/usr/bin/cat", "/usr/bin/mv", "/usr/bin/sleep"] {
        install(root, Path::new(binary), binary);
    }
    std::fs::write(root.join("usr/bin/codex"), include_str!("runc_codex.sh")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join("usr/bin/codex"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

fn install(root: &Path, source: &Path, guest: &str) {
    let target = root.join(guest.trim_start_matches('/'));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // Debug runners carry large symbol tables. Keep private OCI copies small
    // without changing the executable code or shared-library requirements.
    assert!(Command::new("strip")
        .arg("--strip-debug")
        .arg("-o")
        .arg(&target)
        .arg(source)
        .status()
        .unwrap()
        .success());
    let libs = Command::new("ldd").arg(source).output().unwrap();
    assert!(libs.status.success(), "ldd {}", source.display());
    for line in String::from_utf8(libs.stdout).unwrap().lines() {
        assert!(!line.contains("not found"), "{line}");
        if let Some(lib) = line.split_whitespace().find(|s| s.starts_with('/')) {
            let dest = root.join(lib.trim_start_matches('/'));
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(lib, dest).unwrap();
        }
    }
}
