use opencoder_dag::DagClaimedRun;
use opencoder_dag_runtime::sandbox::run::{execute, RunContainer, StepProcess};
use serde_json::json;
use std::{path::Path, process::Command};

#[path = "support/container.rs"]
mod container;

struct BindMounts(Vec<std::path::PathBuf>);

impl BindMounts {
    fn bind(&mut self, source: &Path, target: &Path) {
        let status = Command::new("mount")
            .arg("--bind")
            .arg(source)
            .arg(target)
            .status()
            .unwrap();
        assert!(status.success());
        self.0.push(target.to_path_buf());
    }
}

impl Drop for BindMounts {
    fn drop(&mut self) {
        for target in self.0.iter().rev() {
            let result = Command::new("umount").arg(target).status().unwrap();
            assert!(result.success(), "fixture unmount: {}", target.display());
        }
    }
}

async fn probe(root: &Path, config: &opencoder_core::Config, script: &str) -> (i32, String) {
    let id = format!("runtime-environment-{}", ulid::Ulid::new());
    let run: DagClaimedRun = serde_json::from_value(json!({
        "run_id":id,"dag_id":"runtime-environment","created_at":0,
        "spec":{"name":"runtime-environment","steps":[
            {"name":"probe","kind":{"type":"binary","resource":"fixture@v1","args":[]}}
        ]}
    }))
    .unwrap();
    let owner = RunContainer::start(root, config, &run).await.unwrap();
    let result = execute(
        root,
        StepProcess {
            key: "environment-probe".into(),
            argv: vec!["/usr/bin/python3".into(), "-c".into(), script.into()],
            env: vec![],
            cwd: "/workspace/probe".into(),
            timeout_secs: Some(15),
        },
        tokio_util::sync::CancellationToken::new(),
        None,
    )
    .await;
    owner.cleanup().await.unwrap();
    let state = Command::new("runc")
        .arg("--root")
        .arg(root.join("runc-state"))
        .args(["list", "--format", "json"])
        .output()
        .unwrap();
    assert!(state.status.success());
    let containers: serde_json::Value = serde_json::from_slice(&state.stdout).unwrap();
    assert!(containers.is_null() || containers == json!([]));
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    assert!(!mounts.contains(&root.display().to_string()));
    println!(
        "{}",
        json!({"run_id":id,"root":root,"runc_empty":true,"container_mounts_released":true})
    );
    result.unwrap()
}

#[tokio::test]
async fn real_container_allocates_and_exchanges_data_through_a_pty() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = container::ContainerFixture::open(temp.path());
    let mut config = fixture.config.clone();
    fixture.configure(&mut config);
    assert!(fixture.pool.is_dir());
    let (code, output) = probe(
        &temp.path().join("run"),
        &config,
        r#"
import json, os, pty, select, tty
master, slave = pty.openpty()
try:
    tty.setraw(slave)
    os.write(slave, b'pty-ready')
    assert select.select([master], [], [], 2)[0]
    assert os.read(master, 64) == b'pty-ready'
    assert os.ttyname(slave).startswith('/dev/pts/')
    print(json.dumps({'pty_allocated': True, 'roundtrip': True,
                      'slave': os.ttyname(slave), 'ptmx': os.readlink('/dev/ptmx')}))
finally:
    os.close(master)
    os.close(slave)
"#,
    )
    .await;
    assert_eq!(code, 0, "{output}");
    println!("{output}");
    assert!(output.contains("\"roundtrip\": true"));
}

#[tokio::test]
async fn real_container_reads_nested_knowledge_mounts_but_cannot_write_them() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = container::ContainerFixture::open(temp.path());
    let knowledge = temp.path().join("knowledge");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for path in [&knowledge, &first, &second] {
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("sentinel"), "original").unwrap();
    }
    let file = temp.path().join("mounted-file");
    std::fs::write(&file, "original").unwrap();
    std::fs::write(second.join("file"), "hidden").unwrap();
    let mut mounts = BindMounts(vec![]);
    mounts.bind(&first, &knowledge.join("nested"));
    mounts.bind(&second, &knowledge.join("nested/nested"));
    mounts.bind(&file, &knowledge.join("nested/nested/file"));
    let agents = temp.path().join("agents");
    std::fs::create_dir_all(agents.join("nested")).unwrap();
    mounts.bind(&first, &agents.join("nested"));
    fixture.config.agent.agents_dir = Some(agents);
    fixture.config.dag.knowledge_root = Some(knowledge.clone());
    let (code, output) = probe(
        &temp.path().join("run"),
        &fixture.config,
        r#"
import errno, json, pathlib
root = pathlib.Path('/run/opencoder/knowledge')
checked = []
for relative in ['sentinel', 'nested/sentinel', 'nested/nested/sentinel', 'nested/nested/file']:
    path = root / relative
    assert path.read_text() == 'original', str(path)
    try:
        with path.open('w') as stream:
            stream.write('unexpected')
    except OSError as error:
        assert error.errno == errno.EROFS, (str(path), error)
    else:
        raise AssertionError('knowledge is writable: ' + str(path))
    checked.append(relative)
pathlib.Path('/workspace/probe/result').write_text('workspace stays writable')
assert not pathlib.Path('/run/opencoder/agents/nested/sentinel').exists()
mounts = [line for line in pathlib.Path('/proc/self/mountinfo').read_text().splitlines()
          if '/run/opencoder/knowledge' in line]
print(json.dumps({'readable_and_readonly': checked, 'workspace_writable': True,
                  'unrelated_submount_hidden': True, 'mounts': mounts}))
"#,
    )
    .await;
    // Container read-only remounts must not change the host's source mount flags.
    for path in [&knowledge, &first, &second] {
        assert_eq!(
            std::fs::read_to_string(path.join("sentinel")).unwrap(),
            "original"
        );
        std::fs::write(path.join("host-write"), "allowed").unwrap();
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
    std::fs::write(&file, "host write allowed").unwrap();
    drop(mounts);
    assert_eq!(code, 0, "{output}");
    println!("{output}");
    assert!(output.contains("nested/nested/file"));
}
