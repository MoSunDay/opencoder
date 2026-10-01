use anyhow::{ensure, Context, Result};
use opencoder_agents::{spawn_nfs_server, NfsServerOpts};
use opencoder_dag::DagClaimedRun;
use opencoder_dag_runtime::sandbox::run::{execute, RunContainer, StepProcess};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

fn command(program: &str, args: &[&str]) -> Result<()> {
    let result = Command::new(program).args(args).output()?;
    ensure!(
        result.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

async fn step(
    root: &Path,
    key: &str,
    mode: &str,
    args: &[&str],
    timeout: u64,
) -> Result<(i32, String)> {
    let mut argv = vec!["/workspace/writer/meta/program".into(), mode.into()];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let result = execute(
        root,
        StepProcess {
            key: key.into(),
            argv,
            env: vec![],
            cwd: if mode == "write" {
                "/workspace/writer"
            } else {
                "/workspace/reader"
            }
            .into(),
            timeout_secs: Some(timeout),
        },
        CancellationToken::new(),
        None,
    )
    .await;
    eprintln!("step {key}: {result:?}");
    result
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        arguments.len() == 2,
        "usage: container-check <rootfs> <native-fixture>"
    );
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("writer"))?;
    std::fs::write(source.join("writer/source.txt"), "original\n")?;
    let binary = std::fs::read(&arguments[1])?;
    opencoder_dag_binary::save_binary_version(
        &source.join("binaries"),
        "fixture",
        "acceptance",
        &binary,
    )?;
    let export = spawn_nfs_server(&NfsServerOpts {
        export_root: source.clone(),
        host: "127.0.0.1".into(),
        port: 0,
        read_only: true,
    })?;
    let port = export.local_addr()?.port();
    let mounted = temp.path().join("nfs");
    std::fs::create_dir(&mounted)?;
    command("mount", &["-t", "nfs", "-o", &format!("ro,vers=3,tcp,port={port},mountport={port},nolock,soft,timeo=10,retrans=1,actimeo=0"), "127.0.0.1:/", mounted.to_str().context("mount path")?])?;
    let mut config = opencoder_core::Config::default();
    config.dag.workspace_dir = Some(mounted.clone());
    config.dag.binary_dir = Some(mounted.join("binaries"));
    config.dag.rootfs_dir = Some(PathBuf::from(&arguments[0]));
    let spec = opencoder_dag::decode_spec(&json!({"name":"native-check","steps":[
        {"name":"writer","kind":{"type":"binary","resource":"fixture","args":["write"]}},
        {"name":"reader","depends_on":["writer"],"kind":{"type":"binary","resource":"fixture","args":["read"]}}
    ]})).map_err(anyhow::Error::msg)?;
    let mut starts = Vec::new();
    let result = async {
        for sample in 0..20 {
            let run = DagClaimedRun { run_id: format!("sample-{sample}"), dag_id: "native-check".into(), created_at: 0, spec: spec.clone() };
            let root = temp.path().join(format!("node-{}/1970-01-01/native-check/{}", sample % 2, run.run_id));
            let started = Instant::now();
            opencoder_dag_runtime::resources::freeze(&root, &config, &spec)?;
            let container = RunContainer::start(&root, &config, &run).await?;
            starts.push(started.elapsed().as_millis());
            if sample == 0 {
                let state = Command::new("runc").arg("--root").arg(root.join("runc-state")).args(["state", "dag-run-sample-0"]).output()?;
                let state: serde_json::Value = serde_json::from_slice(&state.stdout)?;
                let pid = state["pid"].as_u64().context("container pid")?;
                eprintln!("host workspace {:?}", std::fs::read_dir(root.join("workspace"))?.map(|entry| entry.map(|entry| entry.file_name())).collect::<Vec<_>>());
                eprintln!("guest workspace {:?}", std::fs::read_dir(format!("/proc/{pid}/root/workspace"))?.map(|entry| entry.map(|entry| entry.file_name())).collect::<Vec<_>>());
                eprintln!("mounts {}", std::fs::read_to_string(format!("/proc/{pid}/mountinfo"))?);
            }
            let checked = async {
                ensure!(step(&root, "writer", "write", &[], 10).await?.0 == 0, "write failed");
                ensure!(step(&root, "reader", "read", &[], 10).await?.0 == 0, "shared workspace failed");
                ensure!(step(&root, "argv", "argv", &["with space", "quoted \"value\"", ""], 10).await?.0 == 0, "argv failed");
                if sample == 0 {
                    ensure!(step(&root, "timeout", "spin", &[], 1).await.is_err(), "timeout not enforced");
                    ensure!(step(&root, "after-timeout", "read", &[], 10).await?.0 == 0, "step timeout stopped the shared container");
                    ensure!(step(&root, "overflow", "overflow", &[], 10).await.is_err(), "output limit not enforced");
                    ensure!(step(&root, "after-overflow", "read", &[], 10).await?.0 == 0, "output overflow stopped the shared container");
                }
                Ok::<_,anyhow::Error>(())
            }.await;
            container.cleanup().await?;
            checked?;
            ensure!(std::fs::read_to_string(source.join("writer/source.txt"))? == "original\n", "source changed");
            ensure!(std::fs::read_to_string(root.join("upper/writer/source.txt"))? == "changed\n", "local write layer missing");
            ensure!(!root.join("runc-state").join(format!("dag-run-{}", run.run_id)).exists(), "container orphan");
        }
        starts.sort_unstable();
        let p95 = starts[18];
        ensure!(p95 <= 10000, "cold-start p95 exceeds 10s: {p95}ms");
        println!("{}", json!({"samples":20,"node_directories":2,"cold_start_p95_ms":p95,"source_unchanged":true,"container_orphans":0}));
        Ok::<_,anyhow::Error>(())
    }.await;
    command("umount", &[mounted.to_str().context("mount path")?])?;
    export.shutdown();
    result
}
