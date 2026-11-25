use opencoder_llm::ChatStream;
use opencoder_worker::{Worker, WorkerOptions};
use serde_json::{json, Value};
use std::{ffi::OsString, path::Path, sync::Arc};

/// Guest path of the runners directory in every DAG sandbox. The private
/// image copy's `usr/bin` is the fallback runners directory, holding the DAG
/// runners beside the fake CLI, so DAG steps reach the CLI there.
const SANDBOX_CODEX: &str = "/run/opencoder/bin/codex";
/// Node-private profile selecting the sandbox CLI for DAG-only Agent cards;
/// host executions keep resolving the CLI through `PATH`.
const SANDBOX_PROFILE: &str = "sandbox";

pub struct Environment {
    values: Vec<(&'static str, Option<OsString>)>,
    _container: super::support::native::container::ContainerFixture,
}
impl Environment {
    pub fn new(root: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("codex"), include_str!("codex.py")).unwrap();
        std::fs::set_permissions(bin.join("codex"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let native = super::support::native::container::ContainerFixture::open(root);
        let agents = agents(root);
        for name in ["act", "plan", "workflow", "explore", "build"] {
            card(&agents, name, "codex");
        }
        // A file agent resolves only through a prompt pool version.
        std::fs::create_dir_all(agents.join("prompts/probe/v1")).unwrap();
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
        sandbox_card(&agents, "matrix-dag");
        let credentials = root.join("credentials");
        std::fs::create_dir_all(&credentials).unwrap();
        std::fs::write(credentials.join("auth.json"), "fixture-login").unwrap();
        // The image is only its `usr/bin`: the DAG runners of the shared
        // fixture plus the fake CLI, which every sandbox binds read-only at
        // `/run/opencoder/bin`.
        let rootfs = root.join("matrix-rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        let copied = std::process::Command::new("cp")
            .arg("-a")
            .arg(native.config.dag.rootfs_dir.as_ref().unwrap().join("."))
            .arg(&rootfs)
            .output()
            .unwrap();
        assert!(
            copied.status.success(),
            "copy matrix test rootfs: {copied:?}"
        );
        std::fs::copy(bin.join("codex"), rootfs.join("usr/bin/codex")).unwrap();
        let pairs = [
            ("PATH", format!("{}:/usr/bin:/bin", bin.display())),
            (
                "OPENCODER_AGENTS_DIR",
                native
                    .config
                    .agent
                    .agents_dir
                    .as_ref()
                    .unwrap()
                    .display()
                    .to_string(),
            ),
            ("CODEX_HOME", credentials.display().to_string()),
            ("DAG_TEST_ROOTFS", rootfs.display().to_string()),
            (
                "MATRIX_CAPTURE",
                credentials.join("capture.jsonl").display().to_string(),
            ),
        ];
        Self {
            values: pairs
                .into_iter()
                .map(|(key, value)| {
                    let old = std::env::var_os(key);
                    std::env::set_var(key, value);
                    (key, old)
                })
                .collect(),
            _container: native,
        }
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in self.values.iter().rev() {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}
pub fn agents(root: &Path) -> std::path::PathBuf {
    root.join("native-resources/source/agents")
}
pub fn card(root: &Path, name: &str, harness: &str) {
    write_card(root, name, harness, None);
}
/// A DAG-only card: its steps run the fake CLI from the runners directory
/// through the sandbox profile instead of the host `PATH`.
pub fn sandbox_card(root: &Path, name: &str) {
    write_card(root, name, "codex", Some(SANDBOX_PROFILE));
}
fn write_card(root: &Path, name: &str, harness: &str, profile: Option<&str>) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    let mut meta = json!({"name":name,"harness":harness});
    if let Some(profile) = profile {
        meta["harness_profile"] = json!(profile);
        // Builtin names resolve without a card; a DAG-only card needs its
        // own prompt reference.
        meta["current"] = json!({"prompt":"probe"});
    }
    std::fs::write(root.join(name).join("meta.json"), meta.to_string()).unwrap();
}
pub fn captures(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("credentials/capture.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub struct MatrixNode {
    worker: Worker,
    _bridge: Option<super::support::native::model::ModelBridge>,
}

impl std::ops::Deref for MatrixNode {
    type Target = Worker;

    fn deref(&self) -> &Self::Target {
        &self.worker
    }
}

pub fn host_pid(record: &Value) -> u32 {
    let pid = record["pid"].as_u64().unwrap().to_string();
    let namespace = record["pid_namespace"].as_str().unwrap();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| {
            let host = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let actual = std::fs::read_link(entry.path().join("ns/pid")).ok()?;
            if actual.to_str() != Some(namespace) {
                return None;
            }
            let status = std::fs::read_to_string(entry.path().join("status")).ok()?;
            let nested = status.lines().find(|line| line.starts_with("NSpid:"))?;
            (nested.split_whitespace().last() == Some(pid.as_str())).then_some(host)
        })
        .expect("captured Codex process must still be running")
}

pub async fn node(root: &Path, client: Option<Arc<dyn ChatStream>>) -> MatrixNode {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    // Empty explicit key wins over inherited host credentials; no native client
    // can be constructed. The test must execute actual Codex fixture processes.
    let agents = std::path::PathBuf::from(std::env::var_os("OPENCODER_AGENTS_DIR").unwrap());
    let mount = agents.parent().unwrap();
    let config = json!({"model":"matrix/model","providers":{"matrix":{"base_url":"http://127.0.0.1:1","api_key":""}},"agent":{"agents_dir":agents,"runtime":{"profiles":{SANDBOX_PROFILE:{"revision":1,"settings":{"executable":SANDBOX_CODEX}}}}},"dag":{"rootfs_dir":std::path::PathBuf::from(std::env::var_os("DAG_TEST_ROOTFS").unwrap()),"workspace_dir":mount.join("workspace"),"binary_dir":mount.join("binaries"),"sandbox":opencoder_core::config::DagSandbox::from_test_env()}});
    std::fs::write(workdir.join(".opencoder/config.json"), config.to_string()).unwrap();
    let mut config = opencoder_core::Config::load(&workdir).unwrap();
    assert!(config.resolve_endpoint().is_err());
    let bridge = client.as_ref().map(|client| {
        let bridge = super::support::native::model::ModelBridge::start(client.clone());
        bridge.configure(&mut config);
        bridge
    });
    std::fs::write(
        workdir.join(".opencoder/config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let worker = Worker::open(
        WorkerOptions {
            name: "codex-matrix".into(),
            workdir,
            data_dir: root.join("node"),
            workflow_root: None,
            max_runs: Some(4),
            dag: true,
        },
        client,
    )
    .await
    .unwrap();
    MatrixNode {
        worker,
        _bridge: bridge,
    }
}
