use opencoder_agents::{spawn_nfs_server, NfsServerHandle, NfsServerOpts};
use opencoder_core::Config;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct ContainerFixture {
    pub pool: PathBuf,
    pub config: Config,
    mount: PathBuf,
    export: Option<NfsServerHandle>,
}

fn run(program: &str, arguments: &[&str]) {
    let result = Command::new(program).args(arguments).output().unwrap();
    assert!(
        result.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

/// Single-source hint for every fail-fast in this fixture: how to prepare
/// the native test image when the default lookup or a freshness check
/// fails.
const ROOTFS_HINT: &str = "prepare the native test image first: scripts/prepare-dag-rootfs.sh target/<profile>/dag-rootfs";

/// Resolve the native test image without `DAG_TEST_ROOTFS`.
///
/// Test binaries run from several layouts (`target/<profile>/deps`,
/// `target/<profile>/build/<pkg>/<hash>/out`, ...), so a fixed
/// `parent().parent()` default only works by accident. Walk a bounded number
/// of ancestors for a `target/<profile>` directory holding a prepared image;
/// when nothing matches, fall back to the profile directory beside the
/// binary so the fail-fast below still points at one concrete path.
fn default_rootfs(executable: &Path) -> PathBuf {
    const MAX_ANCESTORS: usize = 8;
    let mut ancestor = executable;
    for _ in 0..MAX_ANCESTORS {
        let Some(profile) = ancestor.parent() else {
            break;
        };
        if profile.file_name().is_some_and(|name| name == "target") {
            let candidate = ancestor.join("dag-rootfs");
            if candidate.join("usr/bin/dag-runner").is_file() {
                return candidate;
            }
        }
        ancestor = profile;
    }
    executable
        .parent()
        .and_then(Path::parent)
        .map(|build| build.join("dag-rootfs"))
        .expect("test binary has no grandparent directory")
}

pub fn rootfs() -> PathBuf {
    let executable = std::env::current_exe().unwrap();
    let path = std::env::var_os("DAG_TEST_ROOTFS")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_rootfs(&executable));
    assert!(path.is_absolute(), "DAG_TEST_ROOTFS must be absolute");
    for name in ["dag-runner", "agent-step-runner"] {
        let runner = path.join("usr/bin").join(name);
        assert!(runner.is_file(), "{ROOTFS_HINT}: {}", runner.display());
        let output = Command::new(&runner).arg("--build-info").output().unwrap();
        assert!(
            output.status.success(),
            "native test runner build metadata unavailable"
        );
        let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let expected = serde_json::to_value(opencoder_core::version::build_info()).unwrap();
        assert_eq!(
            actual,
            expected,
            "stale native test image: {} — {ROOTFS_HINT}",
            runner.display()
        );
    }
    path
}

impl ContainerFixture {
    pub fn open(root: &Path) -> Self {
        let source = root.join("native-resources/source");
        let pool = source.join("binaries");
        std::fs::create_dir_all(&pool).unwrap();
        std::fs::create_dir_all(source.join("workspace")).unwrap();
        let agents = source.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        let mount = root.join("native-resources/mount");
        std::fs::create_dir_all(&mount).unwrap();
        let mut config = Config::default();
        config.dag.rootfs_dir = Some(rootfs());
        let export = spawn_nfs_server(&NfsServerOpts {
            export_root: source,
            host: "127.0.0.1".into(),
            port: 0,
            read_only: true,
        })
        .unwrap();
        let port = export.local_addr().unwrap().port();
        run("mount", &["-t", "nfs", "-o", &format!("ro,vers=3,tcp,port={port},mountport={port},nolock,soft,timeo=10,retrans=1,actimeo=0,lookupcache=none"), "127.0.0.1:/", mount.to_str().unwrap()]);
        config.dag.workspace_dir = Some(mount.join("workspace"));
        config.dag.binary_dir = Some(mount.join("binaries"));
        config.agent.agents_dir = Some(mount.join("agents"));
        Self {
            pool,
            config,
            mount,
            export: Some(export),
        }
    }

    pub fn configure(&self, config: &mut Config) {
        config.dag.workspace_dir = self.config.dag.workspace_dir.clone();
        config.dag.binary_dir = self.config.dag.binary_dir.clone();
        config.dag.rootfs_dir = self.config.dag.rootfs_dir.clone();
    }
}

impl Drop for ContainerFixture {
    fn drop(&mut self) {
        let result = Command::new("umount").arg(&self.mount).output();
        if !matches!(&result, Ok(output) if output.status.success()) {
            eprintln!(
                "native fixture NFS cleanup failed at {}: {result:?}",
                self.mount.display()
            );
        }
        if let Some(export) = self.export.take() {
            export.shutdown();
        }
    }
}
