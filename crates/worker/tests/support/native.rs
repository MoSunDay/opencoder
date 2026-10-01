use opencoder_llm::ChatStream;
use std::path::Path;
use std::sync::Arc;

#[path = "../../../dag-runtime/tests/support/container.rs"]
pub mod container;
#[path = "../../../dag-runtime/tests/support/model.rs"]
pub mod model;

pub fn environment(
    root: &Path,
    client: Arc<dyn ChatStream>,
) -> (container::ContainerFixture, model::ModelBridge) {
    let workdir = root.join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    let native = container::ContainerFixture::open(&root.join("node"));
    let mut config = opencoder_core::Config::load(&workdir).unwrap();
    let source = config
        .agent
        .agents_dir
        .clone()
        .or_else(opencoder_core::agent::agents_dir)
        .filter(|path| path.exists());
    let agents = native.pool.parent().unwrap().join("agents");
    std::fs::remove_dir(&agents).unwrap();
    opencoder_agents::snapshot::pin(source.as_deref(), &agents).unwrap();
    native.configure(&mut config);
    let bridge = model::ModelBridge::start(client);
    bridge.configure(&mut config);
    config.agent.agents_dir = native.config.agent.agents_dir.clone();
    std::fs::write(
        workdir.join("opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    (native, bridge)
}

pub fn run_root(data: &Path, id: &str) -> std::path::PathBuf {
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("dag").join(id).join("execution.json")).unwrap(),
    )
    .unwrap();
    std::path::PathBuf::from(record["annotations"]["dag_parent"].as_str().unwrap()).join(id)
}

pub fn compile(source: &str) -> Vec<u8> {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("fixture.c");
    let binary = temp.path().join("fixture");
    std::fs::write(&input, source).unwrap();
    let status = std::process::Command::new("cc")
        .args(["-O2", "-static", "-s", "-Wl,--build-id=none"])
        .arg(input)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read(binary).unwrap()
}

pub fn stage_binary(data_dir: &Path, name: &str, source: &str) {
    opencoder_dag_binary::save_binary_version(
        &data_dir.join("native-resources/source/binaries"),
        name,
        "fixture",
        &compile(source),
    )
    .unwrap();
}

pub fn stage_stdout_binary(data_dir: &Path, name: &str, message: &str) {
    let message = serde_json::to_string(message).unwrap();
    stage_binary(
        data_dir,
        name,
        &format!("#include <stdio.h>\nint main(void) {{ puts({message}); return 0; }}"),
    );
}

pub fn stage_spin_binary(data_dir: &Path) {
    stage_binary(
        data_dir,
        "spin",
        "#include <unistd.h>\nint main(void) { for (;;) pause(); }",
    );
}
