//! Native sandbox environment, end to end against the real fleet: a binary
//! step reports the kernel-enforced facts of its sandbox (credentials,
//! capabilities, PID/namespace placement, read-only root, masked paths,
//! devices, mounts), the host sees the same sandbox through `/proc/<init>`
//! while a step is in flight, and a cancelled run releases all of it.

use std::path::Path;

use serde_json::{json, Value};

use crate::clone3_fixtures::{
    assert_released, backend_ready, dir_identity, expected_backends, mount_at, mounts,
    mounts_under, namespace, read_json, status_field, wait_for_live,
};
use crate::clone3_probe::PROBE_C;
use crate::fixtures::{publish, stdout_source, SPIN_C};
use crate::support::fleet_proc::Fleet;
use crate::support::llm_stub::LlmStub;

const EROFS: i64 = 30;

/// The sandbox's credential expectation, derived from the bundle: the node's
/// execution user with the configured bounding set; only root keeps the
/// configured permitted/effective capabilities (a non-root user drops them).
#[derive(Debug, PartialEq, Eq)]
struct Credential {
    uid: u64,
    gid: u64,
    bounding: u64,
    permitted: u64,
    effective: u64,
    ambient: u64,
}

const CAP_BITS: &[(&str, u64)] = &[
    ("CAP_CHOWN", 1),
    ("CAP_DAC_OVERRIDE", 1 << 1),
    ("CAP_DAC_READ_SEARCH", 1 << 2),
    ("CAP_FOWNER", 1 << 3),
    ("CAP_FSETID", 1 << 4),
    ("CAP_KILL", 1 << 5),
    ("CAP_SETGID", 1 << 6),
    ("CAP_SETUID", 1 << 7),
    ("CAP_SETPCAP", 1 << 8),
];

fn cap_mask(list: &Value) -> u64 {
    list.as_array().unwrap().iter().fold(0, |mask, cap| {
        let name = cap.as_str().unwrap();
        let bit = CAP_BITS
            .iter()
            .find(|(known, _)| *known == name)
            .unwrap_or_else(|| panic!("fixture capability {name}"))
            .1;
        mask | bit
    })
}

impl Credential {
    fn of(bundle: &Value) -> Self {
        let caps = &bundle["process"]["capabilities"];
        let uid = bundle["process"]["user"]["uid"].as_u64().unwrap();
        let keep = |set: &Value| if uid == 0 { cap_mask(set) } else { 0 };
        Self {
            uid,
            gid: bundle["process"]["user"]["gid"].as_u64().unwrap(),
            bounding: cap_mask(&caps["bounding"]),
            permitted: keep(&caps["permitted"]),
            effective: keep(&caps["effective"]),
            ambient: keep(&caps["ambient"]),
        }
    }

    fn quad(value: &str) -> String {
        [value; 4].join(" ")
    }

    fn hex(mask: u64) -> String {
        format!("{mask:016x}")
    }

    /// `/proc/<pid>/status` credential lines of a sandbox process.
    fn assert_status(&self, status: &Value, label: &str) {
        assert_eq!(status["uid"], Self::quad(&self.uid.to_string()), "{label}");
        assert_eq!(status["gid"], Self::quad(&self.gid.to_string()), "{label}");
        for (field, mask) in [
            ("cap_bnd", self.bounding),
            ("cap_prm", self.permitted),
            ("cap_eff", self.effective),
            ("cap_amb", self.ambient),
        ] {
            assert_eq!(status[field], Self::hex(mask), "{label}: {status}");
        }
        assert_eq!(status["no_new_privs"], "1", "{label}");
    }

    fn assert_host(&self, pid: i32, label: &str) {
        assert_eq!(
            status_field(pid, "Uid"),
            Self::quad(&self.uid.to_string()),
            "{label}"
        );
        assert_eq!(
            status_field(pid, "Gid"),
            Self::quad(&self.gid.to_string()),
            "{label}"
        );
        assert_eq!(
            status_field(pid, "CapBnd"),
            Self::hex(self.bounding),
            "{label}"
        );
        assert_eq!(
            status_field(pid, "CapPrm"),
            Self::hex(self.permitted),
            "{label}"
        );
        assert_eq!(
            status_field(pid, "CapEff"),
            Self::hex(self.effective),
            "{label}"
        );
        assert_eq!(
            status_field(pid, "CapAmb"),
            Self::hex(self.ambient),
            "{label}"
        );
        assert_eq!(status_field(pid, "NoNewPrivs"), "1", "{label}");
    }
}

fn save(fleet: &Fleet, spec: Value) {
    let (status, body) = fleet.http("POST", "/api/dag/defs", &json!({"spec": spec}));
    assert_eq!(status, 200, "save def: {body}");
}

fn dispatch(fleet: &Fleet, definition: &str, run: &str) {
    let (status, body) = fleet.http(
        "POST",
        &format!("/api/dag/defs/{definition}/dispatch"),
        &json!({"id": run}),
    );
    assert_eq!(status, 202, "dispatch {run}: {body}");
}

fn step_text(run_root: &Path, step: &str, file: &str) -> String {
    std::fs::read_to_string(run_root.join(step).join(file))
        .unwrap_or_else(|error| panic!("read {step}/{file}: {error}"))
}

/// The probe's JSON line from the step's captured output.
fn probe(run_root: &Path, step: &str) -> Value {
    let text = step_text(run_root, step, "output.txt");
    let line = text
        .lines()
        .rev()
        .find(|line| line.starts_with("{\"pid\":"))
        .unwrap_or_else(|| panic!("probe output: {text}"));
    serde_json::from_str(line).unwrap_or_else(|error| panic!("probe JSON: {error}: {line}"))
}

fn seen_ro(mounts: &[Value], point: &str) -> Option<bool> {
    mounts
        .iter()
        .rev()
        .find(|mount| mount["point"] == json!(point))
        .map(|mount| mount["ro"] == json!(true))
}

fn seen_kind(mounts: &[Value], point: &str) -> Option<String> {
    mounts
        .iter()
        .rev()
        .find(|mount| mount["point"] == json!(point))
        .and_then(|mount| mount["kind"].as_str().map(str::to_string))
}

/// The receipt names the backend the harness pinned; the bundle is the same
/// OCI document for both backends.
fn assert_receipt(fleet: &Fleet, run: &str) {
    let run_root = fleet.run_root(run);
    let receipt = read_json(&run_root.join("container.json"));
    assert_eq!(receipt["id"], json!(format!("dag-run-{run}")), "{receipt}");
    assert!(
        expected_backends().contains(&receipt["backend"].as_str().unwrap_or_default()),
        "backend of {receipt}"
    );
    let bundle = read_json(&run_root.join("bundle/config.json"));
    assert_eq!(
        bundle["process"]["args"],
        json!(["/usr/bin/dag-runner", "init"])
    );
    assert_eq!(bundle["root"]["readonly"], json!(true));
    assert_eq!(bundle["process"]["noNewPrivileges"], json!(true));
    let namespaces: Vec<&str> = bundle["linux"]["namespaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect();
    assert_eq!(namespaces, ["pid", "ipc", "uts", "mount"], "no net/user ns");
}

#[test]
fn step_sandbox_is_namespaced_readonly_and_unprivileged() {
    if !backend_ready() {
        return;
    }
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let knowledge = tmp.path().join("knowledge");
    std::fs::create_dir_all(&knowledge).unwrap();
    std::fs::write(knowledge.join("guide.md"), "read-only\n").unwrap();

    let fleet = Fleet::spawn_native(
        tmp.path(),
        stub.port(),
        json!({"dag": {"knowledge_root": knowledge.canonicalize().unwrap()}}),
        "sandbox-environment-node",
    );
    publish(&fleet, "envprobe", PROBE_C);
    let definition = "e2e-sandbox-environment";
    let run = "dag-e2e-sandbox-environment-1";
    save(
        &fleet,
        json!({"name": definition, "steps": [
            {"name": "facts", "kind": {"type":"binary", "resource":"envprobe"}},
        ]}),
    );
    dispatch(&fleet, definition, run);
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "done", "inspect: {doc}");
    assert_receipt(&fleet, run);

    let run_root = fleet.run_root(run);
    let facts = probe(&run_root, "facts");
    eprintln!("sandbox facts: {facts}");
    let credential = Credential::of(&read_json(&run_root.join("bundle/config.json")));

    // The step is the configured execution user with only the configured
    // capability sets, inside the sandbox's PID namespace.
    let status = &facts["status"];
    assert_eq!(facts["uid"], credential.uid);
    assert_eq!(facts["euid"], credential.uid);
    assert_eq!(facts["gid"], credential.gid);
    assert_eq!(facts["egid"], credential.gid);
    credential.assert_status(status, "step");
    assert_eq!(
        credential.bounding, 0xb,
        "the bounding set is the fixture's"
    );
    let nspid = status["nspid"].as_str().unwrap();
    assert_eq!(
        nspid.parse::<u64>().unwrap(),
        facts["pid"].as_u64().unwrap()
    );
    assert_ne!(nspid, "1", "the step must not be the sandbox init");

    // PID 1 is the rootfs supervisor: same credentials, pid 1 of the namespace.
    assert_eq!(facts["init_comm"], "dag-runner");
    credential.assert_status(&facts["init_status"], "init");
    assert_eq!(facts["init_status"]["nspid"], "1");

    // Namespaces: the step shares the init's; mount/pid/uts/ipc are private
    // to the sandbox, the network stays the node's.
    for kind in ["mnt", "pid", "uts", "ipc", "net", "user", "cgroup"] {
        assert_eq!(facts["ns"][kind], facts["init_ns"][kind], "{kind}");
    }
    let me = std::process::id();
    for kind in ["mnt", "pid", "uts", "ipc"] {
        assert_ne!(
            facts["ns"][kind].as_u64().unwrap(),
            namespace(me, kind),
            "{kind} namespace leaked from the host"
        );
    }
    assert_eq!(facts["ns"]["net"].as_u64().unwrap(), namespace(me, "net"));
    assert_eq!(facts["hostname"], "dag-run");
    // Both backends materialise the same hostname as a rootfs file: the node
    // writes it into the run's overlay before the sandbox starts.
    assert_eq!(facts["etc_hostname"], "dag-run");

    // Masked and read-only paths, writable scratch and workspace.
    assert_eq!(facts["kcore_size"], 0, "kcore must be masked");
    assert_eq!(facts["dev_null"], json!(true));
    assert_eq!(facts["dev_ptmx"], json!(true));
    assert_eq!(facts["ptmx_link"], "pts/ptmx");
    let rdonly = &facts["rdonly"];
    let write = &facts["write"];
    for path in ["root", "etc", "knowledge"] {
        assert_eq!(rdonly[path], 1, "{path} must be read-only: {rdonly}");
        assert_eq!(write[path], EROFS, "{path} accepted a write: {write}");
    }
    // procfs answers a missing entry with ENOENT on its read-only sys tree;
    // what matters is that the create was refused.
    assert_eq!(rdonly["proc_sys"], 1, "{rdonly}");
    assert!(
        matches!(write["proc_sys"].as_i64(), Some(EROFS) | Some(2)),
        "{write}"
    );
    for path in ["tmp", "workspace"] {
        assert_eq!(rdonly[path], 0, "{rdonly}");
        assert_eq!(write[path], 0, "{write}");
    }

    // The mount table the step sees.
    let seen = facts["mounts"].as_array().unwrap();
    assert_eq!(seen_kind(seen, "/").as_deref(), Some("overlay"));
    assert_eq!(seen_ro(seen, "/"), Some(true));
    assert_eq!(seen_kind(seen, "/proc").as_deref(), Some("proc"));
    assert_eq!(seen_kind(seen, "/tmp").as_deref(), Some("tmpfs"));
    assert_eq!(seen_ro(seen, "/tmp"), Some(false));
    assert_eq!(seen_kind(seen, "/dev").as_deref(), Some("tmpfs"));
    assert_eq!(seen_kind(seen, "/dev/pts").as_deref(), Some("devpts"));
    assert_eq!(seen_ro(seen, "/workspace"), Some(false));
    for path in [
        "/proc/sys",
        "/run/opencoder/knowledge",
        "/run/opencoder-private",
        "/workspace/facts/meta",
    ] {
        assert_eq!(seen_ro(seen, path), Some(true), "mount {path}: {seen:?}");
    }

    // Released once done: the recorded sandbox processes are gone and
    // nothing of the run is mounted any more (the OCI backend keeps no
    // process record once its container is deleted).
    assert_released(
        &run_root,
        crate::clone3_fixtures::recorded(&run_root).as_ref(),
        fleet.agent_pid(),
    );
    assert_eq!(stub.request_count(), 0);
}

#[test]
fn live_sandbox_is_visible_from_the_host_and_released_on_cancel() {
    if !backend_ready() {
        return;
    }
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_native(tmp.path(), stub.port(), json!({}), "sandbox-live-node");
    publish(&fleet, "hold", SPIN_C);
    publish(&fleet, "after", &stdout_source("after"));
    let definition = "e2e-sandbox-live";
    let run = "dag-e2e-sandbox-live-1";
    save(
        &fleet,
        json!({"name": definition, "steps": [
            {"name": "hold", "kind": {"type":"binary", "resource":"hold"}},
            {"name": "after", "depends_on": ["hold"],
             "kind": {"type":"binary", "resource":"after"}},
        ]}),
    );
    dispatch(&fleet, definition, run);
    fleet.wait_status(run, "hold step running", 30, |doc| {
        doc["dag_steps"]["running"] == 1
    });
    assert_receipt(&fleet, run);

    // Host view of the live init: pid 1 of a private PID namespace, running
    // unprivileged from the run's bundle rootfs.
    let run_root = fleet.run_root(run);
    let live = wait_for_live(&run_root);
    let me = std::process::id();
    assert_eq!(
        status_field(live.init.pid, "NSpid"),
        format!("{} 1", live.init.pid)
    );
    assert_eq!(status_field(live.init.pid, "Name"), "dag-runner");
    Credential::of(&read_json(&run_root.join("bundle/config.json")))
        .assert_host(live.init.pid, "init");
    assert_eq!(
        dir_identity(Path::new(&format!("/proc/{}/root", live.init.pid))),
        dir_identity(&run_root.join("bundle/rootfs")),
        "the init's root is not the run's bundle rootfs"
    );
    for kind in ["mnt", "pid", "uts", "ipc"] {
        assert_ne!(namespace(live.init.pid as u32, kind), namespace(me, kind));
    }
    assert_eq!(namespace(live.init.pid as u32, "net"), namespace(me, "net"));

    // The init's mount table: the OCI mounts of the bundle, root read-only.
    let table = mounts(live.init.pid as u32);
    assert_eq!(mount_at(&table, "/").kind, "overlay");
    assert!(mount_at(&table, "/").read_only());
    assert_eq!(mount_at(&table, "/proc").kind, "proc");
    assert_eq!(mount_at(&table, "/tmp").kind, "tmpfs");
    assert_eq!(mount_at(&table, "/dev").kind, "tmpfs");
    let pts = mount_at(&table, "/dev/pts");
    assert_eq!(pts.kind, "devpts");
    assert!(!pts.read_only());
    assert!(!mount_at(&table, "/workspace").read_only());
    for point in [
        "/proc/sys",
        "/run/opencoder-private",
        "/workspace/hold/meta",
    ] {
        assert!(mount_at(&table, point).read_only(), "{point}: {table:?}");
    }

    // The node's own mount table carries exactly the run's two overlays.
    let agent = fleet.agent_pid();
    let mut expected = vec![
        run_root.join("bundle/rootfs").display().to_string(),
        run_root.join("workspace").display().to_string(),
    ];
    expected.sort();
    assert_eq!(mounts_under(agent, &run_root), expected);

    // The native launcher keeps the lease from the host's PID namespace.
    if let Some(launcher) = live.launcher {
        assert_eq!(
            status_field(launcher.pid, "NSpid"),
            launcher.pid.to_string()
        );
        assert_eq!(namespace(launcher.pid as u32, "pid"), namespace(me, "pid"));
        assert_ne!(namespace(launcher.pid as u32, "mnt"), namespace(me, "mnt"));
    }

    // Cancel: the in-flight step drains, the run folds to cancelled and the
    // sandbox is gone.
    let (status, body) = fleet.http("POST", &format!("/api/dag/runs/{run}/cancel"), &json!({}));
    assert_eq!(status, 200, "cancel: {body}");
    assert_eq!(body["status"], "cancelling");
    let doc = fleet.wait_terminal(run);
    assert_eq!(doc["execution"]["status"], "cancelled", "fold: {doc}");
    let meta = read_json(&run_root.join("hold/meta.json"));
    assert_eq!(meta["outcome"], "cancelled", "{meta}");
    assert_released(&run_root, Some(&live), agent);
    assert_eq!(mounts_under(agent, &run_root), Vec::<String>::new());
}
