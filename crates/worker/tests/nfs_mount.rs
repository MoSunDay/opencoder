mod support;
use opencoder_agents::serve::{spawn_nfs_server, NfsServerHandle, NfsServerOpts};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::{path::PathBuf, process::Command};
use support::*;

fn resource(source: &std::path::Path, category: &str, name: &str, path: &str, body: &str) {
    let root = source.join(category).join(name);
    let file = root.join("v1").join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        root.join("meta.json"),
        json!({"name":name,"current":1}).to_string(),
    )
    .unwrap();
    std::fs::write(file, body).unwrap();
}

struct Export {
    mount: PathBuf,
    handle: Option<NfsServerHandle>,
    mounted: bool,
}
impl Export {
    fn unmount(&mut self) {
        let result = Command::new("timeout")
            .args(["10", "umount"])
            .arg(&self.mount)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "umount: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        self.mounted = false;
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        if self.mounted {
            let _ = Command::new("timeout")
                .args(["10", "umount", "-l"])
                .arg(&self.mount)
                .status();
        }
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
    }
}

/// Explicitly run on the release validation host; never silently skip missing
/// privileges or NFS support. Only this test's temporary export is mounted.
#[tokio::test]
#[ignore = "manual: needs mount privileges and NFS client"]
async fn readonly_nfs_node_snapshots_and_offline_followup() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("export");
    let mount = dir.path().join("mount");
    std::fs::create_dir_all(&mount).unwrap();
    resource(&source, "prompts", "review", "soul.md", "version one");
    resource(
        &source,
        "skills",
        "review-skills",
        "alpha/SKILL.md",
        "pinned skill",
    );
    let deep = "alpha/references/a-complete-service-contract-with-a-long-name.md";
    resource(
        &source,
        "skills",
        "review-skills",
        deep,
        "complete deep reference",
    );
    resource(&source, "tools", "review-tools", "check", "pinned tool");
    resource(
        &source,
        "memory",
        "review-memory",
        "memory.md",
        "pinned memory",
    );
    std::fs::create_dir_all(source.join("reviewer")).unwrap();
    std::fs::write(
        source.join("reviewer/meta.json"),
        json!({"name":"reviewer","current":{
            "prompt":"review",
            "skills":"review-skills",
            "tools":"review-tools",
            "memory":"review-memory"
        }})
        .to_string(),
    )
    .unwrap();
    let handle = spawn_nfs_server(&NfsServerOpts {
        export_root: source.clone(),
        host: "127.0.0.1".into(),
        port: 0,
        read_only: true,
    })
    .unwrap();
    let port = handle.local_addr().unwrap().port();
    let mut export = Export {
        mount: mount.clone(),
        handle: Some(handle),
        mounted: false,
    };
    let result = Command::new("timeout").args(["10", "mount", "-t", "nfs", "-o"])
        .arg(format!("ro,vers=3,tcp,port={port},mountport={port},nolock,soft,retrans=1,timeo=10,actimeo=0,lookupcache=none"))
        .arg("127.0.0.1:/").arg(&mount).output().unwrap();
    assert!(
        result.status.success(),
        "mount: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    export.mounted = true;
    assert!(std::fs::write(mount.join("should-not-exist"), "blocked").is_err());
    let node_root = dir.path().join("node-fixture");
    std::fs::create_dir_all(node_root.join("work")).unwrap();
    std::fs::write(
        node_root.join("work/opencoder.json"),
        json!({"agent":{"agents_dir":mount}}).to_string(),
    )
    .unwrap();
    let node = worker(&node_root, mock()).await;
    assert!(node.snapshot().ready, "{:?}", node.snapshot());
    let assign = |id| {
        let mut request = assignment(&node, id, ExecutionKind::Agent, json!({}), None);
        request.request.target = Some("reviewer".into());
        request
    };
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assign("agent-v1")
        })
        .await
        .status,
        200
    );
    settled(&node, "agent-v1").await;
    std::fs::create_dir_all(source.join("prompts/review/v2")).unwrap();
    std::fs::write(source.join("prompts/review/v2/soul.md"), "version two").unwrap();
    std::fs::write(
        source.join("prompts/review/meta.json"),
        json!({"name":"review","current":2}).to_string(),
    )
    .unwrap();
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assign("agent-v2")
        })
        .await
        .status,
        200
    );
    settled(&node, "agent-v2").await;
    let second_root = dir.path().join("node-fixture-b");
    std::fs::create_dir_all(second_root.join("work")).unwrap();
    std::fs::write(
        second_root.join("work/opencoder.json"),
        json!({"agent":{"agents_dir":mount}}).to_string(),
    )
    .unwrap();
    let second = worker(&second_root, mock()).await;
    let mut second_assignment = assignment(
        &second,
        "agent-node-b",
        ExecutionKind::Agent,
        json!({}),
        None,
    );
    second_assignment.request.target = Some("reviewer".into());
    assert_eq!(
        second
            .handle(NodeOperation::Create {
                assignment: second_assignment,
            })
            .await
            .status,
        200
    );
    settled(&second, "agent-node-b").await;
    for (id, version, text) in [
        ("agent-v1", 1, "version one"),
        ("agent-v2", 2, "version two"),
    ] {
        let pinned = node_root.join(format!(
            "node/agent/{id}/resources/prompts/review/v{version}/soul.md"
        ));
        assert_eq!(std::fs::read_to_string(pinned).unwrap(), text);
    }
    let pinned = node_root.join("node/agent/agent-v1/resources");
    assert_eq!(
        std::fs::read_to_string(pinned.join("skills/review-skills/v1").join(deep)).unwrap(),
        "complete deep reference"
    );
    for (relative, text) in [
        ("skills/review-skills/v1/alpha/SKILL.md", "pinned skill"),
        ("tools/review-tools/v1/check", "pinned tool"),
        ("memory/review-memory/v1/memory.md", "pinned memory"),
    ] {
        assert_eq!(
            std::fs::read_to_string(pinned.join(relative)).unwrap(),
            text
        );
    }
    assert!(pinned.join("reviewer/meta.json").is_file());
    // The real kernel client retains its mount and directory/file handles.
    export.handle.take().unwrap().shutdown();
    export.handle = Some(
        spawn_nfs_server(&NfsServerOpts {
            export_root: source.clone(),
            host: "127.0.0.1".into(),
            port,
            read_only: true,
        })
        .unwrap(),
    );
    assert_eq!(
        std::fs::read_to_string(mount.join("skills/review-skills/v1").join(deep)).unwrap(),
        "complete deep reference"
    );
    export.unmount();
    assert!(!node.snapshot().ready);
    assert!(!second.snapshot().ready);
    let reply = prompt(&node, "agent-v1", "continue using pinned resources").await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let detail = settled(&node, "agent-v1").await;
    assert_eq!(detail["execution"]["status"], "idle", "{detail}");
    let reply = node
        .handle(NodeOperation::Create {
            assignment: assign("agent-unmounted"),
        })
        .await;
    assert_eq!(reply.status, 400, "{reply:?}");
    assert!(!source.join("executions").exists());
    assert!(!source.join("runtime.db").exists());
    assert!(node_root.join("node/runtime.db").is_file());
    assert!(second_root.join("node/runtime.db").is_file());
    assert!(node_root
        .join("node/agent/agent-v1/execution.json")
        .is_file());
    assert!(!node_root.join("node/agent/agent-node-b").exists());
    assert!(second_root
        .join("node/agent/agent-node-b/execution.json")
        .is_file());
    assert!(!second_root.join("node/agent/agent-v1").exists());
    node.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}
