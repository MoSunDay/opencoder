use opencoder_llm::ChatStream;
use opencoder_worker::{Worker, WorkerOptions};
use serde_json::{json, Value};
use std::{ffi::OsString, path::Path, sync::Arc};

pub struct Environment(Vec<(&'static str, Option<OsString>)>);
impl Environment {
    pub fn new(root: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("codex"), include_str!("codex.py")).unwrap();
        std::fs::set_permissions(bin.join("codex"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let agents = root.join("agents");
        for name in ["act", "plan", "workflow", "explore", "build"] {
            card(&agents, name, "codex");
        }
        let pairs = [
            ("PATH", format!("{}:/usr/bin:/bin", bin.display())),
            ("OPENCODER_AGENTS_DIR", agents.display().to_string()),
            (
                "MATRIX_CAPTURE",
                root.join("capture.jsonl").display().to_string(),
            ),
        ];
        Self(
            pairs
                .into_iter()
                .map(|(key, value)| {
                    let old = std::env::var_os(key);
                    std::env::set_var(key, value);
                    (key, old)
                })
                .collect(),
        )
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        for (key, value) in self.0.iter().rev() {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}
pub fn card(root: &Path, name: &str, harness: &str) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    std::fs::write(
        root.join(name).join("meta.json"),
        json!({"name":name,"harness":harness}).to_string(),
    )
    .unwrap();
}
pub fn captures(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("capture.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub async fn node(root: &Path, client: Option<Arc<dyn ChatStream>>) -> Worker {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    // Empty explicit key wins over inherited host credentials; no native client
    // can be constructed. The test must execute actual Codex fixture processes.
    let config = json!({"model":"matrix/model","providers":{"matrix":{"base_url":"http://127.0.0.1:1","api_key":""}},"agent":{"agents_dir":null},"ap":{"mode":"off"}});
    std::fs::write(workdir.join(".opencoder/config.json"), config.to_string()).unwrap();
    let config = opencoder_core::Config::load(&workdir).unwrap();
    assert!(config.resolve_endpoint().is_err());
    assert!(
        config.agent.agents_dir.is_none(),
        "{:?}",
        config.agent.agents_dir
    );
    Worker::open(
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
    .unwrap()
}
