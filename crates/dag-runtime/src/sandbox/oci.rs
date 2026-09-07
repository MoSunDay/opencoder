//! OCI bundle generation for `sandbox: runc` wasm steps.
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
pub struct BundleSpec {
    /// `<workflow_root>/<run_id>` — bind-mounted rw at `/workspace/context`
    /// so the step reads upstream artifacts and writes its own under
    /// `/workspace/context/<step>/output.json`.
    pub run_root: PathBuf,
    /// Step slug (annotations + step dir naming).
    pub step_slug: String,
    /// The wasm launch command argv (module token first). The module path
    /// is resolved inside the container against `/workspace/context`.
    pub command: Vec<String>,
    /// Extra env pairs injected into the container process (the
    /// `OPENCODER_*` step contract).
    pub env: Vec<(String, String)>,
    /// Wall-clock budget hint recorded in `annotations`; the actual kill is
    /// performed by the runc runner, not by the container itself.
    pub timeout_hint: Option<u64>,
}

/// Where the shared read-only rootfs lives for a given run root:
/// `<workflow_root>/rootfs` (sibling of `<workflow_root>/<run_id>`).
pub fn shared_rootfs(run_root: &Path) -> Result<PathBuf> {
    let workflow_root = run_root
        .parent()
        .context("run root has no parent (workflow root)")?;
    Ok(workflow_root.join("rootfs"))
}

/// The OCI `config.json` for one wasm step.
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
            "args": wasm_args(spec),
            "env": wasm_env(spec),
            "cwd": "/workspace",
        },
        "root": { "path": "rootfs", "readonly": true },
        "mounts": [
            {
                "destination": "/proc",
                "type": "proc",
                "source": "proc",
            },
            {
                "destination": "/tmp",
                "type": "tmpfs",
                "source": "tmpfs",
                "options": ["rw", "nosuid", "nodev", "size=64m"],
            },
            {
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
            },
        ],
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

/// Container argv: run the module with the static `wasmtime` CLI —
/// `wasmtime run --dir=<context> [--env K=V]... <module> [args...]`. The
/// module token is rewritten to its guest-visible path under
/// `/workspace/context`; later tokens pass through verbatim.
fn wasm_args(spec: &BundleSpec) -> Vec<String> {
    let mut args = vec![
        "wasmtime".to_string(),
        "run".to_string(),
        format!("--dir={}", crate::exec::wasm::CONTEXT_MOUNT),
    ];
    for (k, v) in &spec.env {
        args.push("--env".to_string());
        args.push(format!("{k}={v}"));
    }
    for (i, token) in spec.command.iter().enumerate() {
        if i == 0 {
            args.push(format!(
                "{}/{}",
                crate::exec::wasm::CONTEXT_MOUNT,
                token.trim_start_matches('/')
            ));
        } else {
            args.push(token.clone());
        }
    }
    args
}

/// Container process env: the standard PATH plus the `OPENCODER_*` step
/// contract pairs.
fn wasm_env(spec: &BundleSpec) -> Vec<String> {
    let mut env = vec!["PATH=/usr/local/bin:/usr/bin:/bin".to_string()];
    env.extend(spec.env.iter().map(|(k, v)| format!("{k}={v}")));
    env
}

/// Materialize the bundle at `dir`: `config.json` referencing the wasm
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
            "shared rootfs unusable at {}: it must be a REAL directory (missing, or a symlink — runc rejects symlinks; move/copy the tree or bind-mount it). Run `opencoder-agent dag prepare-rootfs` / place a static wasmtime tree there",
            shared.display()
        );
    }

    // 1. A real, private root isolates runc device initialization and pins
    //    runtime files for retries. The source is never modified.
    super::rootfs::snapshot(&shared, &dir)?;

    // 2. config.json.
    let config = container_config(spec);
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
        // The bind-mount destination: runc does NOT auto-create mount
        // points inside the rootfs — a missing dir fails container init
        // with a confusing "no such device" ENODEV.
        "workspace/context",
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
# DAG sandbox rootfs scaffold

This directory is the provisioned rootfs template for `sandbox: runc` wasm
steps. It is a scaffold: you must add a wasm runtime before runc can run
anything.

1. Place a STATIC `wasmtime` CLI tree under `usr/` (build wasmtime with
   `cargo build --release` on the target arch and copy the binary plus any
   needed runtime libs so that `/usr/bin/wasmtime` resolves inside the
   rootfs). A static build avoids glibc-version coupling with the host.
2. Keep `etc/resolv.conf` in sync if your host resolver setup changes
   (copied from the host by `dag prepare-rootfs`; the sandbox shares the
   host network — there is no network namespace).
3. `dev`, `proc`, `sys`, `tmp` and `workspace/context` are recreated as
   empty runtime directories in each private copy. `proc` and `tmp` are
   mounted at runtime; workspace/context receives the run artifacts bind.
4. Point `<workflow_root>/rootfs` at this tree. It must end up a REAL
   directory — runc rejects symlinked rootfs paths ("invalid rootfs: not an
   absolute path, or a symlink"), so move/copy the tree there or bind-mount
   it (`mount --bind <tree> <workflow_root>/rootfs`). Every step bundle
   copies the runtime tree into its own real `rootfs` directory, used
   read-only at runtime and reused for retries. Reserve disk space for
   one private runtime tree per step; no shared files are hard-linked.

No network downloads happen at prepare or step time in the runtime itself —
populating `usr/` is a provisioning concern. The wasm module itself is NOT
part of the rootfs: it lives in the run artifacts and reaches the container
through the `/workspace/context` bind mount.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(workflow_root: &Path) -> BundleSpec {
        BundleSpec {
            run_root: workflow_root.join("run-1"),
            step_slug: "step-a".into(),
            command: vec!["step-a/main.wasm".into(), "--flag".into()],
            env: vec![("OPENCODER_RUN_ID".into(), "run-1".into())],
            timeout_hint: Some(30),
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
        // Args run the module via the static wasmtime CLI with the context
        // dir preopened and the step env forwarded; later tokens verbatim.
        let args = cfg["process"]["args"].as_array().unwrap();
        assert_eq!(args[0], "wasmtime");
        assert_eq!(args[1], "run");
        assert_eq!(args[2], "--dir=/workspace/context");
        assert_eq!(args[3], "--env");
        assert_eq!(args[4], "OPENCODER_RUN_ID=run-1");
        assert_eq!(args[5], "/workspace/context/step-a/main.wasm");
        assert_eq!(args[6], "--flag");
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

        // config.json references the module inside the context bind via the
        // static wasmtime CLI (the module itself was staged by the executor).
        let cfg: Value =
            serde_json::from_str(&fs::read_to_string(bundle.join("config.json")).unwrap()).unwrap();
        assert_eq!(cfg["process"]["args"][0], "wasmtime");
        assert_eq!(
            cfg["process"]["args"][5],
            "/workspace/context/step-a/main.wasm"
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
        ] {
            assert!(out.join(sub).is_dir(), "missing {sub}");
        }
        let readme = fs::read_to_string(out.join("README.md")).unwrap();
        assert!(
            readme.contains("usr/"),
            "readme explains wasmtime placement"
        );
        assert!(readme.contains("resolv.conf"));
    }
}
