//!
//! JSON/path plumbing and bundle preparation; process driving lives in
//! [`super::runc`]. Each bundle takes a private copy of the provisioned
//! `<workflow_root>/rootfs`, preserving its interpreter version and isolating
//! the device files that runc initializes before making the root read-only.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use serde_json::{json, Value};

/// Everything needed to render one step's OCI bundle.
#[derive(Clone)]
pub struct BundleSpec {
    /// `<workflow_root>/<run_id>` — bind-mounted rw at `/workspace/context`
    /// so the step reads upstream artifacts and writes its own under
    /// `/workspace/context/<step>/output.json`.
    pub run_root: PathBuf,
    /// Step slug (annotations + step dir naming).
    pub step_slug: String,
    /// is resolved inside the container against `/workspace/context`.
    pub command: Vec<String>,
    /// Extra env pairs injected into the container process (the
    /// `OPENCODER_*` step contract).
    pub env: Vec<(String, String)>,
    /// Wall-clock budget hint recorded in `annotations`; the actual kill is
    /// performed by the runc runner, not by the container itself.
    pub timeout_hint: Option<u64>,
    /// Optional read-only knowledge mount: the node's knowledge root
    /// bind-mounted READ-ONLY at `/workspace/knowledge`. Kernel-enforced:
    /// steps can read the tree but never modify it.
    pub knowledge: Option<KnowledgeMount>,
    /// Optional read-only bind of a pinned agents pool at `/workspace/agent`
    /// (host path): agent cards + the four shared pools, kernel-enforced ro.
    /// Agent-session workloads set this; DAG steps leave it `None`.
    pub agents: Option<PathBuf>,
}

/// A read-only bind of a host knowledge tree into the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeMount {
    /// Host path (absolute after [`write_bundle`] normalization; validated
    /// as a REAL directory, fail-closed).
    pub host: PathBuf,
}

/// Where the shared read-only rootfs lives for a given run root:
/// `<workflow_root>/rootfs` (sibling of `<workflow_root>/<run_id>`).
pub fn shared_rootfs(run_root: &Path) -> Result<PathBuf> {
    let workflow_root = run_root
        .parent()
        .context("run root has no parent (workflow root)")?;
    Ok(workflow_root.join("rootfs"))
}

///
/// Notes on the (deliberate) shape:
/// - `ociVersion` stays at `"1.0.0"` — the most widely accepted value across
///   runc releases.
/// - rootfs is **readonly**; writable mounts are `/workspace/context`,
///   and a fresh `/tmp`. Device initialization uses the private root tree.
/// - namespaces: pid + ipc + uts + mount. **No network namespace** on purpose:
///   host networking keeps the sandbox dependency-free (no bridge/veth setup);
///   hostname only takes effect because of the uts namespace.
/// - `terminal: false`, uid/gid 0 — minimal static runtime trees we expect
///   under `usr/` have everything world-readable; the mount namespace plus
///   readonly root is the isolation boundary here, not uid dropping.
pub fn container_config(spec: &BundleSpec) -> Value {
    let bind_source = spec
        .run_root
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    let mut annotations = serde_json::Map::new();
    annotations.insert(
        "org.opencoder.dag.step".to_string(),
        Value::String(spec.step_slug.clone()),
    );
    // OCI annotations are strictly map[string]string — number-typed values
    // make runc reject the whole config at parse time.
    if let Some(secs) = spec.timeout_hint {
        annotations.insert(
            "org.opencoder.dag.timeout_secs".to_string(),
            Value::String(secs.to_string()),
        );
    }
    json!({
        "ociVersion": "1.0.0",
        "annotations": Value::Object(annotations),
        "hostname": "dag-step",
        "process": {
            "terminal": false,
            "user": { "uid": 0, "gid": 0 },
            "args": spec.command.clone(),
            "env": process_env(spec),
            "cwd": "/workspace",
        },
        "root": { "path": "rootfs", "readonly": true },
        "mounts": mounts(spec, bind_source),
        "linux": {
            "namespaces": [
                { "type": "pid" },
                { "type": "ipc" },
                { "type": "uts" },
                { "type": "mount" },
            ],
        },
    })
}

/// The mounts array: proc + fresh /tmp + the rw context bind, plus the
/// OPTIONAL read-only knowledge and agents-pool binds. Built imperatively
/// because `json!` keeps `null` placeholders inside arrays.
fn mounts(spec: &BundleSpec, bind_source: String) -> Value {
    let mut mounts = vec![
        json!({
            "destination": "/proc",
            "type": "proc",
            "source": "proc",
        }),
        json!({
            "destination": "/tmp",
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["rw", "nosuid", "nodev", "size=64m"],
        }),
        json!({
            "destination": "/workspace/context",
            "type": "bind",
            "source": bind_source,
            // "rbind" (MS_REC|MS_BIND): some kernels refuse the plain
            // legacy MS_BIND path through runc's fd-based mount helper
            // with ENODEV ("no such device") — the recursive variant
            // goes through open_tree/move_mount and works everywhere
            // we tested. The source is a plain dir (no submounts), so
            // semantics are identical.
            "options": ["rw", "rbind"],
        }),
    ];
    if let Some(knowledge) = knowledge_mount(spec) {
        mounts.push(knowledge);
    }
    if let Some(agents) = agents_mount(spec) {
        mounts.push(agents);
    }
    Value::Array(mounts)
}

/// The read-only knowledge bind entry, or `None` when no knowledge root is
/// configured. `ro` + `rbind`: the kernel enforces read-only — a step can
/// read the node's knowledge tree but never modify it.
fn knowledge_mount(spec: &BundleSpec) -> Option<Value> {
    let knowledge = spec.knowledge.as_ref()?;
    let host = knowledge
        .host
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    Some(json!({
        "destination": crate::exec::KNOWLEDGE_MOUNT,
        "type": "bind",
        "source": host,
        "options": ["ro", "rbind"],
    }))
}

/// The read-only agents-pool bind entry, or `None` when no pinned pool is
/// requested (DAG steps). Same `ro` + `rbind` shape as the knowledge mount:
/// the kernel enforces read-only, so agent cards and the four shared
/// resource pools are visible to the session runner but immutable.
fn agents_mount(spec: &BundleSpec) -> Option<Value> {
    let host = spec
        .agents
        .as_ref()?
        .to_string_lossy()
        .trim_end_matches('/')
        .to_string();
    Some(json!({
        "destination": crate::exec::AGENTS_MOUNT,
        "type": "bind",
        "source": host,
        "options": ["ro", "rbind"],
    }))
}

fn process_env(spec: &BundleSpec) -> Vec<String> {
    let mut env = vec!["PATH=/usr/local/bin:/usr/bin:/bin".to_string()];
    env.extend(spec.env.iter().map(|(k, v)| format!("{k}={v}")));
    env
}

/// Validate + absolutize the knowledge host path, returning an owned spec.
fn normalize_knowledge(spec: &BundleSpec) -> Result<BundleSpec> {
    let Some(knowledge) = &spec.knowledge else {
        return Ok(spec.clone());
    };
    let abs = std::path::absolute(&knowledge.host)
        .with_context(|| format!("knowledge_root {}", knowledge.host.display()))?;
    let is_real_dir = fs::symlink_metadata(&abs)
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    if !is_real_dir {
        bail!(
            "knowledge_root unusable at {}: it must be a REAL directory (missing or a symlink)",
            knowledge.host.display()
        );
    }
    let mut normalized = spec.clone();
    normalized.knowledge = Some(KnowledgeMount { host: abs });
    Ok(normalized)
}

/// module under the bind-mounted `/workspace/context` (the executor
/// already wrote the module-adjacent step artifacts). The shared rootfs is
/// validated and copied into the bundle; subsequent attempts reuse that
/// private runtime tree.
/// Returns `dir` as an absolute path on success.
pub fn write_bundle(dir: &Path, spec: &BundleSpec) -> Result<PathBuf> {
    let dir = std::path::absolute(dir)?;
    fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;

    // Fail closed when the shared rootfs tree is missing — or is a
    // symlink: runc rejects symlinked rootfs paths outright ("invalid
    // rootfs: not an absolute path, or a symlink"), so a symlinked shared
    // tree must fail here with an actionable message instead of at
    // container start. Provisioning must move/copy the tree or bind-mount
    // it (a real directory is required).
    let shared = shared_rootfs(&spec.run_root)?;
    let is_real_dir = fs::symlink_metadata(&shared)
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    if !is_real_dir {
        bail!(
            "shared rootfs unusable at {}: it must be a REAL directory (missing, or a symlink — runc rejects symlinks; move/copy the tree or bind-mount it). Run `opencoder-agent dag prepare-rootfs`",
            shared.display()
        );
    }

    // 0. Knowledge root: fail closed BEFORE any bundle work when the
    //    configured tree is missing or a symlink — a knowledge mount that
    //    silently no-ops (or points elsewhere) breaks the read-only
    //    contract the node promised. Returns an owned, host-path-normalized
    //    spec so `container_config` emits an absolute bind source.
    let spec = normalize_knowledge(spec)?;

    // 1. A real, private root isolates runc device initialization and pins
    //    runtime files for retries. The source is never modified.
    let rootfs = super::rootfs::snapshot(&shared, &dir)?;
    // The knowledge mountpoint inside the private root tree (runc would
    // create it under the readonly root; pre-creating keeps the config
    // self-contained and mount failure modes explicit).
    if spec.knowledge.is_some() {
        fs::create_dir_all(rootfs.join(crate::exec::KNOWLEDGE_MOUNT.trim_start_matches('/')))
            .with_context(|| format!("mkdir knowledge mountpoint under {}", rootfs.display()))?;
    }
    // The pinned agents-pool mountpoint: same pre-creation discipline —
    // runc does not auto-create mountpoints under the readonly root.
    if spec.agents.is_some() {
        fs::create_dir_all(rootfs.join(crate::exec::AGENTS_MOUNT.trim_start_matches('/')))
            .with_context(|| format!("mkdir agents mountpoint under {}", rootfs.display()))?;
    }
    // 2. config.json.
    let config = container_config(&spec);
    let config_path = dir.join("config.json");
    fs::write(&config_path, serde_json::to_vec_pretty(&config)?)
        .with_context(|| format!("write {}", config_path.display()))?;

    Ok(dir)
}

/// Scaffold a rootfs template at `out` — the backend of
/// `opencoder-agent dag prepare-rootfs`. Creates the documented directory
/// skeleton + README and copies the host resolv.conf when present; it does
/// NOT download anything (no network use at prepare time).
pub fn write_rootfs_template(out: &Path) -> Result<()> {
    for sub in [
        "dev",
        "proc",
        "sys",
        "etc",
        "tmp",
        "usr/bin",
        "usr/lib",
        // The bind-mount destinations: runc does NOT auto-create mount
        // points inside the rootfs — a missing dir fails container init
        // with a confusing "no such device" ENODEV. `workspace/agent` is
        // the pinned read-only agents pool of agent-session workloads.
        "workspace/context",
        "workspace/agent",
    ] {
        let dir = out.join(sub);
        fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }

    // Host network (no netns) — a resolv.conf in the image keeps DNS working
    // for steps that reach the network from inside the sandbox.
    if Path::new("/etc/resolv.conf").is_file() {
        let dest = out.join("etc/resolv.conf");
        let _ = fs::copy("/etc/resolv.conf", &dest);
    }

    fs::write(
        out.join("README.md"),
        README_TEMPLATE.trim_start_matches('\n'),
    )
    .with_context(|| format!("write {}", out.join("README.md").display()))?;
    Ok(())
}

const README_TEMPLATE: &str = r#"
# Native runtime rootfs

Install dag-runner, agent-step-runner and agent-session-runner under usr/bin
using scripts/prepare-dag-rootfs.sh. Provision required native tools and
libraries; never place credentials in this image. Keep etc/resolv.conf valid.
DAG runs use an OverlayFS private runtime view and one shared /workspace.
Standalone Agent sessions use their own OCI bundles.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(workflow_root: &Path) -> BundleSpec {
        BundleSpec {
            run_root: workflow_root.join("run-1"),
            step_slug: "step-a".into(),
            command: vec!["/usr/bin/native-tool".into(), "--flag".into()],
            env: vec![("OPENCODER_RUN_ID".into(), "run-1".into())],
            timeout_hint: Some(30),
            knowledge: None,
            agents: None,
        }
    }

    #[test]
    fn container_config_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = container_config(&spec(tmp.path()));

        assert_eq!(cfg["ociVersion"], "1.0.0");
        assert_eq!(cfg["hostname"], "dag-step");
        // OCI resolves this real root directory relative to the bundle.
        assert_eq!(cfg["root"]["path"], "rootfs");
        assert_eq!(cfg["root"]["readonly"], true);
        let mounts = cfg["mounts"].as_array().unwrap();
        let bind = mounts
            .iter()
            .find(|m| m["type"] == "bind")
            .expect("bind mount present");
        assert_eq!(bind["destination"], "/workspace/context");
        assert_eq!(
            bind["source"],
            tmp.path()
                .join("run-1")
                .to_string_lossy()
                .trim_end_matches('/')
        );
        let opts = bind["options"].as_array().unwrap();
        assert!(opts.contains(&json!("rw")), "{opts:?}");
        assert!(opts.contains(&json!("rbind")), "{opts:?}");
        assert_eq!(
            cfg["process"]["args"],
            json!(["/usr/bin/native-tool", "--flag"])
        );
        let env = cfg["process"]["env"].as_array().unwrap();
        assert!(env.contains(&json!("OPENCODER_RUN_ID=run-1")), "{env:?}");
        assert_eq!(cfg["process"]["cwd"], "/workspace");
        assert_eq!(cfg["process"]["terminal"], false);
        // Namespace set: pid/ipc/uts/mount, no network.
        let ns: Vec<&str> = cfg["linux"]["namespaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(ns, vec!["pid", "ipc", "uts", "mount"]);
        assert!(!ns.contains(&"network"));
        // Timeout hint lands in annotations (OCI annotations are strings).
        assert_eq!(cfg["annotations"]["org.opencoder.dag.timeout_secs"], "30");
        assert_eq!(cfg["annotations"]["org.opencoder.dag.step"], "step-a");
    }

    #[test]
    fn write_bundle_writes_config_and_private_rootfs() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow_root = tmp.path().join("workflow");
        // Shared rootfs pre-exists (as the runner guarantees).
        fs::create_dir_all(workflow_root.join("rootfs")).unwrap();

        let bundle =
            write_bundle(&workflow_root.join("run-1.bundle"), &spec(&workflow_root)).unwrap();
        assert!(bundle.is_absolute());

        let cfg: Value =
            serde_json::from_str(&fs::read_to_string(bundle.join("config.json")).unwrap()).unwrap();
        assert_eq!(
            cfg["process"]["args"],
            json!(["/usr/bin/native-tool", "--flag"])
        );
        assert_eq!(cfg["root"]["path"], "rootfs");
        assert!(fs::symlink_metadata(bundle.join("rootfs"))
            .unwrap()
            .is_dir());
        assert!(bundle.join("rootfs/workspace/context").is_dir());
        assert!(!workflow_root.join("rootfs/workspace").exists());
    }

    #[test]
    fn write_bundle_fails_closed_without_shared_rootfs() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow_root = tmp.path().join("workflow");
        fs::create_dir_all(&workflow_root).unwrap(); // no rootfs subdir
        let err = write_bundle(&workflow_root.join("b"), &spec(&workflow_root)).unwrap_err();
        assert!(
            err.to_string().contains("shared rootfs unusable"),
            "{err:#}"
        );
        assert!(err.to_string().contains("runc rejects symlinks"), "{err:#}");
    }

    #[test]
    fn write_bundle_rejects_symlinked_shared_rootfs() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow_root = tmp.path().join("workflow");
        fs::create_dir_all(workflow_root.join("real")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("real", workflow_root.join("rootfs")).unwrap();
        let err = write_bundle(&workflow_root.join("b"), &spec(&workflow_root)).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err:#}");
    }

    #[test]
    fn write_rootfs_template_creates_documented_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("rootfs");
        write_rootfs_template(&out).unwrap();
        for sub in [
            "dev",
            "proc",
            "sys",
            "etc",
            "tmp",
            "usr/bin",
            "usr/lib",
            "workspace",
            "workspace/context",
            "workspace/agent",
        ] {
            assert!(out.join(sub).is_dir(), "missing {sub}");
        }
        let readme = fs::read_to_string(out.join("README.md")).unwrap();
        assert!(
            readme.contains("usr/"),
            "readme explains executable placement"
        );
        assert!(readme.contains("resolv.conf"));
    }

    #[test]
    fn knowledge_mount_shapes_config_args_and_mountpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let knowledge = tmp.path().join("kb");
        fs::create_dir_all(&knowledge).unwrap();
        // run_root under a workflow dir whose shared rootfs pre-exists, so
        // the write_bundle section below is exercisable with the same spec.
        let workflow_root = tmp.path().join("workflow");
        fs::create_dir_all(workflow_root.join("rootfs")).unwrap();
        let mut this = spec(&workflow_root);
        this.knowledge = Some(KnowledgeMount {
            host: knowledge.clone(),
        });

        let cfg = container_config(&this);
        let mounts = cfg["mounts"].as_array().unwrap();
        let kb = mounts
            .iter()
            .find(|m| m["destination"] == json!("/workspace/knowledge"))
            .expect("knowledge bind present");
        assert_eq!(kb["type"], "bind");
        assert_eq!(kb["source"], knowledge.to_string_lossy().as_ref());
        let opts = kb["options"].as_array().unwrap();
        assert!(opts.contains(&json!("ro")), "{opts:?}");
        assert!(opts.contains(&json!("rbind")), "{opts:?}");
        assert_eq!(
            cfg["process"]["args"],
            json!(["/usr/bin/native-tool", "--flag"])
        );

        // write_bundle normalizes the host path, creates the mountpoint in
        // the private root tree, and keeps it out of the shared one.
        let bundle = write_bundle(&workflow_root.join("run-1.bundle"), &this).unwrap();
        let cfg2: Value =
            serde_json::from_str(&fs::read_to_string(bundle.join("config.json")).unwrap()).unwrap();
        let kb2 = cfg2["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["destination"] == json!("/workspace/knowledge"))
            .unwrap();
        assert_eq!(kb2["source"], knowledge.to_string_lossy().as_ref());
        assert!(bundle.join("rootfs/workspace/knowledge").is_dir());
    }

    #[test]
    fn knowledge_mount_absent_without_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = container_config(&spec(tmp.path()));
        let mounts = cfg["mounts"].as_array().unwrap();
        assert!(!mounts
            .iter()
            .any(|m| m["destination"] == json!("/workspace/knowledge")));
        let args = cfg["process"]["args"].as_array().unwrap();
        assert!(
            !args
                .iter()
                .any(|a| a.as_str().unwrap_or("").contains("knowledge")),
            "{args:?}"
        );
    }

    #[test]
    fn agents_mount_shapes_config_and_mountpoint() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tmp.path().join("agents-pool");
        fs::create_dir_all(&agents).unwrap();
        // run_root under a workflow dir whose shared rootfs pre-exists, so
        // the write_bundle section below is exercisable with the same spec.
        let workflow_root = tmp.path().join("workflow");
        fs::create_dir_all(workflow_root.join("rootfs")).unwrap();
        let mut this = spec(&workflow_root);
        this.agents = Some(agents.clone());

        let cfg = container_config(&this);
        let mounts = cfg["mounts"].as_array().unwrap();
        let bind = mounts
            .iter()
            .find(|m| m["destination"] == json!("/workspace/agent"))
            .expect("agents bind present");
        assert_eq!(bind["type"], "bind");
        assert_eq!(bind["source"], agents.to_string_lossy().as_ref());
        let opts = bind["options"].as_array().unwrap();
        assert!(opts.contains(&json!("ro")), "{opts:?}");
        assert!(opts.contains(&json!("rbind")), "{opts:?}");
        // The agents pool serves the agent-session runner (Direct argv):
        let args = cfg["process"]["args"].as_array().unwrap();
        assert!(
            !args
                .iter()
                .any(|a| a.as_str().unwrap_or("").contains("workspace/agent")),
            "{args:?}"
        );

        // write_bundle creates the mountpoint in the private root tree and
        // keeps it out of the shared one.
        let bundle = write_bundle(&workflow_root.join("run-1.bundle"), &this).unwrap();
        let cfg2: Value =
            serde_json::from_str(&fs::read_to_string(bundle.join("config.json")).unwrap()).unwrap();
        let ag2 = cfg2["mounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["destination"] == json!("/workspace/agent"))
            .unwrap();
        assert_eq!(ag2["source"], agents.to_string_lossy().as_ref());
        assert!(bundle.join("rootfs/workspace/agent").is_dir());
        assert!(!workflow_root.join("rootfs/workspace").exists());
    }

    #[test]
    fn agents_mount_absent_without_configuration() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = container_config(&spec(tmp.path()));
        let mounts = cfg["mounts"].as_array().unwrap();
        assert!(!mounts
            .iter()
            .any(|m| m["destination"] == json!("/workspace/agent")));
        let args = cfg["process"]["args"].as_array().unwrap();
        assert!(
            !args
                .iter()
                .any(|a| a.as_str().unwrap_or("").contains("agent")),
            "{args:?}"
        );
    }

    #[test]
    fn knowledge_mount_fails_closed_on_missing_or_symlink_root() {
        let tmp = tempfile::tempdir().unwrap();
        let workflow_root = tmp.path().join("workflow");
        fs::create_dir_all(workflow_root.join("rootfs")).unwrap();
        let mut missing = spec(&workflow_root);
        missing.knowledge = Some(KnowledgeMount {
            host: workflow_root.join("no-such-kb"),
        });
        let err = write_bundle(&workflow_root.join("b1"), &missing).unwrap_err();
        assert!(
            err.to_string().contains("knowledge_root unusable"),
            "{err:#}"
        );

        // A symlinked knowledge root is rejected the same way.
        fs::create_dir_all(workflow_root.join("real-kb")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("real-kb", workflow_root.join("kb-link")).unwrap();
        let mut linked = spec(&workflow_root);
        linked.knowledge = Some(KnowledgeMount {
            host: workflow_root.join("kb-link"),
        });
        let err = write_bundle(&workflow_root.join("b2"), &linked).unwrap_err();
        assert!(
            err.to_string().contains("knowledge_root unusable"),
            "{err:#}"
        );
    }

    #[test]
    fn direct_argv_passes_command_through_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let mut this = spec(tmp.path());
        this.command = vec!["/usr/bin/agent-step-runner".into()];
        let cfg = container_config(&this);
        let args = cfg["process"]["args"].as_array().unwrap();
        assert_eq!(
            serde_json::to_value(args).unwrap(),
            json!(["/usr/bin/agent-step-runner"])
        );
        // Env pairs still ride the process env list.
        assert!(cfg["process"]["env"]
            .as_array()
            .unwrap()
            .contains(&json!("OPENCODER_RUN_ID=run-1")));
    }
}
