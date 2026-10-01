use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct NativeMounts {
    mounts: Vec<(PathBuf, &'static str)>,
    pub node_config: Value,
    pub server_config: Value,
}

pub fn copy_rootfs(target: &Path) {
    let source = std::env::var_os("DAG_TEST_ROOTFS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            super::sibling_bin(super::SERVER_BIN)
                .parent()
                .unwrap()
                .join("dag-rootfs")
        });
    for name in ["dag-runner", "agent-step-runner", "agent-session-runner"] {
        assert!(
            source.join("usr/bin").join(name).is_file(),
            "prepare the paired test image before running tests"
        );
    }
    std::fs::create_dir_all(target).unwrap();
    let result = Command::new("cp")
        .arg("-a")
        .arg(source.join("."))
        .arg(target)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "copy test rootfs: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

impl NativeMounts {
    pub fn prepare(workdir: &Path, extra: Value) -> Self {
        let rootfs = std::env::var_os("DAG_TEST_ROOTFS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                super::sibling_bin(super::SERVER_BIN)
                    .parent()
                    .unwrap()
                    .join("dag-rootfs")
            });
        assert!(rootfs.is_absolute(), "DAG_TEST_ROOTFS must be absolute");
        for name in ["dag-runner", "agent-step-runner"] {
            assert!(
                rootfs.join("usr/bin").join(name).is_file(),
                "prepare native test rootfs: scripts/prepare-dag-rootfs.sh {}",
                rootfs.display()
            );
        }
        let mut server = extra.clone();
        if !server.is_object() {
            server = json!({});
        }
        let mut node = extra;
        if !node.is_object() {
            node = json!({});
        }
        let mut mounts = vec![];
        for (section, key, label) in [
            ("dag", "binary_dir", "binaries"),
            ("dag", "workspace_dir", "workspace"),
            ("agent", "agents_dir", "agents"),
        ] {
            let source = server[section][key]
                .as_str()
                .map(PathBuf::from)
                .unwrap_or_else(|| workdir.join("native-source").join(label));
            let mount = workdir.join("native-mounts").join(label);
            std::fs::create_dir_all(&source).unwrap();
            std::fs::create_dir_all(&mount).unwrap();
            if server.get(section).is_none() {
                server[section] = json!({});
            }
            if node.get(section).is_none() {
                node[section] = json!({});
            }
            server[section][key] = json!(source);
            server[section][if key == "workspace_dir" {
                "workspace_nfs"
            } else {
                "nfs"
            }] = json!({"enabled":true,"host":"127.0.0.1","port":0,"read_only":true});
            if key == "workspace_dir" {
                server[section]["workspace_nfs"]
                    .as_object_mut()
                    .unwrap()
                    .remove("read_only");
            }
            node[section][key] = json!(mount);
            let endpoint = match label {
                "binaries" => "/api/dag/binaries/nfs",
                "workspace" => "/api/dag/workspace/nfs",
                _ => "/api/agents/nfs",
            };
            mounts.push((mount, endpoint));
        }
        node["dag"]["rootfs_dir"] = json!(rootfs);
        node["dag"]["data_dir"] = json!(workdir.join("node-state/dag/runs"));
        Self {
            mounts,
            node_config: node,
            server_config: server,
        }
    }

    pub fn mount(&self, base: &str) {
        for (mount, endpoint) in &self.mounts {
            let (status, response) =
                super::http_util::http(base, "GET", endpoint, super::fleet_proc::TOKEN, "");
            assert_eq!(status, 200, "NFS status: {response}");
            assert_eq!(
                response["status"]["running"], true,
                "NFS status: {response}"
            );
            let listener = u16::try_from(response["status"]["port"].as_u64().unwrap()).unwrap();
            assert_ne!(listener, 0, "NFS listener must already be bound");
            let output = Command::new("mount").args(["-t", "nfs", "-o"])
                .arg(format!("ro,vers=3,tcp,port={listener},mountport={listener},nolock,soft,timeo=10,retrans=1,actimeo=0,lookupcache=none"))
                .arg("127.0.0.1:/").arg(mount).output().unwrap();
            assert!(
                output.status.success(),
                "NFS mount: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

impl Drop for NativeMounts {
    fn drop(&mut self) {
        for (mount, _) in self.mounts.iter().rev() {
            let output = Command::new("umount").arg(mount).output().unwrap();
            if !output.status.success() {
                eprintln!(
                    "NFS cleanup {}: {}",
                    mount.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}
