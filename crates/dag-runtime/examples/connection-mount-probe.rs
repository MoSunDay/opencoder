//! Isolated real-runc verification of explicitly declared private connections.
use anyhow::{ensure, Context, Result};
use opencoder_core::harness::{CodexSettings, Versioned};
use opencoder_dag_runtime::sandbox::codex;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    fs::create_dir_all(path.parent().context("file parent missing")?)?;
    fs::write(path, contents)?;
    Ok(())
}

fn main() -> Result<()> {
    let parent = std::env::current_dir()?.join("target/tmp");
    fs::create_dir_all(&parent)?;
    let temporary = tempfile::Builder::new()
        .prefix("connection-mount-probe-")
        .tempdir_in(parent)?;
    let root = temporary.path().canonicalize()?;
    let rootfs = root.join("rootfs");
    let program = root.join("launcher/run");
    write(&program, "#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755))?;
    write(
        &rootfs.join(program.strip_prefix("/")?),
        fs::read(&program)?,
    )?;
    fs::set_permissions(
        rootfs.join(program.strip_prefix("/")?),
        fs::Permissions::from_mode(0o755),
    )?;
    write(&rootfs.join("usr/bin/codex"), "#!/bin/sh\nexit 0\n")?;
    fs::set_permissions(
        rootfs.join("usr/bin/codex"),
        fs::Permissions::from_mode(0o755),
    )?;
    write(
        &rootfs.join("usr/share/opencoder/codex-startup.json"),
        json!({"schema_version":1,"program":program,"codex":"/usr/bin/codex",
            "files":{"run":format!("{:x}",Sha256::digest(fs::read(&program)?))}})
        .to_string(),
    )?;
    let busybox = Path::new("/usr/bin/busybox");
    write(&rootfs.join("bin/busybox"), fs::read(busybox)?)?;
    fs::set_permissions(
        rootfs.join("bin/busybox"),
        fs::Permissions::from_mode(0o755),
    )?;
    let libraries = Command::new("ldd").arg(busybox).output()?;
    ensure!(
        libraries.status.success(),
        "busybox dependency lookup failed"
    );
    for library in String::from_utf8(libraries.stdout)?
        .split_whitespace()
        .filter(|value| value.starts_with('/'))
    {
        let destination = rootfs.join(Path::new(library).strip_prefix("/")?);
        write(&destination, fs::read(library)?)?;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))?;
    }
    let home = root.join("home");
    fs::create_dir_all(&home)?;
    write(&root.join("proxy"), "http://fixture.invalid")?;
    write(&root.join("deployment.json"), "{}")?;
    write(&root.join("clients/opencoder/ctl.json"), "{}")?;
    let connection = root.join("connections/service.json");
    let token = root.join("connections/token");
    write(&connection, json!({"token_file":"token"}).to_string())?;
    write(&token, "fixture-private-value\n")?;
    let hidden = root.join("connections/undeclared.json");
    write(&hidden, "{}")?;
    let settings_path = root.join("startup.json");
    write(
        &settings_path,
        json!({"deployment_config":root.join("deployment.json"),
            "proxy_file":root.join("proxy"),"codex_home":home,
            "sqlite_home":home.join("sqlite"),"client_config_home":root.join("clients"),
            "codex":"/usr/bin/codex","connection_files":[connection]})
        .to_string(),
    )?;
    let agents = root.join("agents");
    write(
        &agents.join("probe/meta.json"),
        json!({"name":"probe","harness":"codex","harness_profile":"probe"}).to_string(),
    )?;
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(agents);
    config.agent.runtime.profiles.insert(
        "probe".into(),
        Versioned {
            revision: 1,
            settings: CodexSettings {
                startup_script: vec![
                    program.display().to_string(),
                    "--config".into(),
                    settings_path.display().to_string(),
                ],
                envs: [("CODEX_HOME".into(), home.display().to_string())].into(),
                ..Default::default()
            },
        },
    );
    let launch = codex::resolve(&config, "probe", &rootfs)?.context("Codex launch missing")?;
    let managed = launch.startup.context("managed startup missing")?;
    let runtime = root.join("runtime");
    fs::create_dir_all(runtime.join("private/codex/probe"))?;
    let mut mounts = vec![];
    managed.prepare(&runtime, "probe", &rootfs, &mut mounts)?;
    ensure!(mounts.len() == 6, "unexpected private mount count");
    let private_mounts = mounts.len();
    fs::create_dir_all(rootfs.join("proc"))?;
    mounts.push(json!({"destination":"/proc","type":"proc","source":"proc"}));
    let bundle = root.join("bundle");
    let script = r#"
        test -r "$1"
        read -r value < "$2"
        test "$value" = fixture-private-value
        test ! -e "$3"
        if (printf changed > "$2"); then exit 30; fi
        if (printf changed > "$1"); then exit 31; fi
        printf 'declared files readable; writes rejected; undeclared file absent\n'
    "#;
    write(
        &bundle.join("config.json"),
        json!({"ociVersion":"1.0.0","root":{"path":rootfs,"readonly":true},
            "process":{"terminal":false,"user":{"uid":0,"gid":0},"cwd":"/",
                "args":["/bin/busybox","sh","-eu","-c",script,"probe",connection,token,hidden],
                "env":["PATH=/bin"]},"mounts":mounts,
            "linux":{"namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"}]}})
        .to_string(),
    )?;
    let state = root.join("runc-state");
    let output = Command::new("runc")
        .arg("--root")
        .arg(&state)
        .args(["run", "--no-pivot", "--bundle"])
        .arg(&bundle)
        .arg("connection-mount-probe")
        .output()?;
    ensure!(
        output.status.success(),
        "runc probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        fs::read(&token)? == b"fixture-private-value\n",
        "token changed"
    );
    ensure!(
        fs::read(&connection)? == json!({"token_file":"token"}).to_string().as_bytes(),
        "connection changed"
    );
    let listed = Command::new("runc")
        .arg("--root")
        .arg(state)
        .args(["list", "--format", "json"])
        .output()?;
    ensure!(listed.status.success(), "runc list failed");
    let containers: Option<Vec<serde_json::Value>> = serde_json::from_slice(&listed.stdout)?;
    let containers = containers.unwrap_or_default();
    ensure!(containers.is_empty(), "probe container was not cleaned up");
    println!(
        "{}",
        json!({"status":"PASS","real_runc":true,"private_mounts":private_mounts,
        "read_only":true,"undeclared_file_absent":true,"source_files_unchanged":true,
        "containers_remaining":0})
    );
    Ok(())
}
