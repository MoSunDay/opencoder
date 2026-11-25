//! Shared helpers of the native-sandbox (`clone3`) e2e scenarios, written
//! against both backends so the same pins run in the `runc` matrix: the
//! backend the harness pins (`DAG_TEST_SANDBOX`) and its skip policy,
//! host-side views of a live sandbox (`/proc/<init>/...`, the backend's own
//! records), the proof that a released run owns nothing, and the compiled
//! in-sandbox environment probe.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use opencoder_core::config::DagSandbox;
use serde_json::Value;

/// Backend the harness configures every node with (`DAG_TEST_SANDBOX`).
pub fn backend() -> DagSandbox {
    DagSandbox::from_test_env()
}

/// Backends `container.json` may name for the harness setting (`auto` is
/// resolved on the node).
pub fn expected_backends() -> &'static [&'static str] {
    match backend() {
        DagSandbox::Clone3 => &["clone3"],
        DagSandbox::Auto => &["clone3", "runc"],
        DagSandbox::Runc => &["runc"],
    }
}

fn runc_available() -> bool {
    Command::new("runc")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Skip policy of the dual-backend scenarios: the pinned native backend
/// needs nothing from the host, the other settings need `runc`.
pub fn backend_ready() -> bool {
    match backend() {
        DagSandbox::Clone3 => true,
        _ if runc_available() => true,
        _ => {
            eprintln!("SKIP: runc unavailable and DAG_TEST_SANDBOX is not pinned to clone3");
            false
        }
    }
}

/// Scenarios that only exist for the native backend skip elsewhere.
pub fn clone3_only(label: &str) -> bool {
    if backend() == DagSandbox::Clone3 {
        return true;
    }
    eprintln!("SKIP {label}: requires DAG_TEST_SANDBOX=clone3");
    false
}

pub fn read_json(path: &Path) -> Value {
    let bytes =
        std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// One exact host process incarnation: pid plus `/proc/<pid>/stat` starttime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub pid: i32,
    pub start_time: u64,
}

/// Identity of a live (non-zombie) pid; `None` once it is gone or reused.
pub fn identity(pid: i32) -> Option<Identity> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(") ")?.1;
    let mut fields = tail.split_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    // Field 22 (starttime): 19 tokens after the state token of the tail.
    let start_time = fields.nth(18)?.parse().ok()?;
    Some(Identity { pid, start_time })
}

/// A live sandbox as the host sees it.
#[derive(Clone, Copy, Debug)]
pub struct Live {
    pub init: Identity,
    /// Native backend only: the launcher process holding the sandbox lease.
    pub launcher: Option<Identity>,
}

fn native_live(record: &Value) -> Option<Live> {
    let who = |key: &str| {
        Some(Identity {
            pid: record[key]["pid"].as_i64()? as i32,
            start_time: record[key]["start_time"].as_u64()?,
        })
    };
    Some(Live {
        init: who("init")?,
        launcher: who("launcher"),
    })
}

/// runc's view of the run's container state, parsed from `runc list`.
pub fn runc_list(run_root: &Path) -> Option<Value> {
    let output = Command::new("runc")
        .arg("--root")
        .arg(run_root.join("runc-state"))
        .args(["list", "--format", "json"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| serde_json::from_slice(&output.stdout).ok())
        .flatten()
}

/// Wait until the run's sandbox is up and return its identities from the
/// backend's own record: the launcher's `init.json`, or `runc list`.
pub fn wait_for_live(run_root: &Path) -> Live {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let native = std::fs::read(run_root.join("private/clone3/init.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|record| native_live(&record));
        let oci = runc_list(run_root)
            .and_then(|list| list.as_array().and_then(|rows| rows.first().cloned()))
            .and_then(|row| row["pid"].as_i64())
            .and_then(|pid| identity(i32::try_from(pid).unwrap()));
        if let Some(live) = native.or_else(|| {
            oci.map(|init| Live {
                init,
                launcher: None,
            })
        }) {
            return live;
        }
        assert!(
            Instant::now() < deadline,
            "sandbox of {} never came up",
            run_root.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The identities a finished run last recorded (the record outlives it).
pub fn recorded(run_root: &Path) -> Option<Live> {
    std::fs::read(run_root.join("private/clone3/init.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|record| native_live(&record))
}

/// Wait until every recorded process of the sandbox is gone.
pub fn wait_gone(live: &Live, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let alive = Some(live.init)
            .into_iter()
            .chain(live.launcher)
            .any(|who| identity(who.pid) == Some(who));
        if !alive {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label}: sandbox processes survived 30 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Backend-specific proof that a released run owns nothing: the recorded
/// processes are gone, `runc` holds no state, and nothing under the run
/// root stays mounted in the node's mount table.
pub fn assert_released(run_root: &Path, live: Option<&Live>, agent: Option<u32>) {
    if let Some(live) = live {
        wait_gone(live, "released run");
    }
    if backend() != DagSandbox::Clone3 {
        let list = runc_list(run_root).unwrap_or(Value::Null);
        assert!(
            list.is_null() || list.as_array().is_some_and(Vec::is_empty),
            "runc state of a released run: {list}"
        );
    }
    let left = mounts_under(agent, run_root);
    assert!(
        left.is_empty(),
        "mounts under {}: {left:?}",
        run_root.display()
    );
}

/// Value of a `/proc/<pid>/status` field, whitespace collapsed
/// (`NSpid` → `"4242 1"`).
pub fn status_field(pid: i32, field: &str) -> String {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/status: {error}"));
    status_value(&text, field)
}

fn status_value(status: &str, field: &str) -> String {
    status
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// Namespace id: the `[inode]` of `/proc/<pid>/ns/<kind>`.
pub fn namespace(pid: u32, kind: &str) -> u64 {
    let target = std::fs::read_link(format!("/proc/{pid}/ns/{kind}"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/ns/{kind}: {error}"));
    let text = target.to_string_lossy();
    text[text.find('[').unwrap() + 1..text.len() - 1]
        .parse()
        .unwrap()
}

/// Device and inode of a path, so a sandbox init's `/` (reached through
/// the `/proc/<pid>/root` magic link) is compared with the run's bundle
/// rootfs by identity: a root runc bind-mounted inside its private mount
/// namespace has no host path to compare.
pub fn dir_identity(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let meta =
        std::fs::metadata(path).unwrap_or_else(|error| panic!("stat {}: {error}", path.display()));
    (meta.dev(), meta.ino())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub point: String,
    pub kind: String,
    pub options: Vec<String>,
}

impl Mount {
    pub fn read_only(&self) -> bool {
        self.options.iter().any(|option| option == "ro")
    }
}

/// Mount table of a host pid (`/proc/<pid>/mountinfo`).
pub fn mounts(pid: u32) -> Vec<Mount> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/mountinfo"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/mountinfo: {error}"));
    parse_mounts(&text)
}

/// Mount points under `root` in the agent's mount table (this process's
/// once the agent is gone: the node mounts in the host namespace).
pub fn mounts_under(agent: Option<u32>, root: &Path) -> Vec<String> {
    let table = agent
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/mountinfo")).ok())
        .or_else(|| std::fs::read_to_string("/proc/self/mountinfo").ok())
        .map(|text| parse_mounts(&text))
        .unwrap_or_default();
    let prefix = format!("{}/", root.display().to_string().trim_end_matches('/'));
    let mut points: Vec<String> = table
        .into_iter()
        .map(|mount| mount.point)
        .filter(|point| point.starts_with(&prefix))
        .collect();
    points.sort();
    points.dedup();
    points
}

fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let left: Vec<&str> = left.split_whitespace().collect();
            Some(Mount {
                point: (*left.get(4)?).to_string(),
                options: left.get(5)?.split(',').map(str::to_string).collect(),
                kind: right.split_whitespace().next()?.to_string(),
            })
        })
        .collect()
}

/// The last mount stacked at `point` (the one a lookup actually reaches).
pub fn mount_at<'a>(mounts: &'a [Mount], point: &str) -> &'a Mount {
    mounts
        .iter()
        .rev()
        .find(|mount| mount.point == point)
        .unwrap_or_else(|| panic!("no mount at {point}: {mounts:?}"))
}

/// `PATH` of a node that cannot resolve any external tool, `runc` included:
/// an empty fixture directory (the node needs nothing from `PATH`).
pub fn path_without_tools(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    dir.display().to_string()
}

/// Step that finishes late enough to be interrupted by a node restart.
pub const LATE_C: &str =
    "#include <stdio.h>\n#include <unistd.h>\nint main(void) { sleep(8); puts(\"late\"); return 0; }";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_collapse_whitespace_and_mounts_parse_both_halves() {
        let status = "Name:\tdag-runner\nNSpid:\t4242\t1\nCapBnd:\t000000000000000b\n";
        assert_eq!(status_value(status, "NSpid"), "4242 1");
        assert_eq!(status_value(status, "CapBnd"), "000000000000000b");
        assert_eq!(status_value(status, "Missing"), "");
        let mounts = parse_mounts(
            "29 24 0:22 / /tmp rw,nosuid - tmpfs tmpfs rw\n\
             30 24 0:22 /run/opencoder-private /run/opencoder-private ro,nosuid,nodev - ext4 /dev/vda1 rw\n",
        );
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].point, "/tmp");
        assert_eq!(mounts[0].kind, "tmpfs");
        assert!(!mounts[0].read_only());
        assert_eq!(mounts[1].point, "/run/opencoder-private");
        assert_eq!(mounts[1].kind, "ext4");
        assert!(mounts[1].read_only());
        assert_eq!(mount_at(&mounts, "/tmp").kind, "tmpfs");
    }

    #[test]
    fn identity_tracks_the_live_test_process_and_rejects_others() {
        let me = std::process::id() as i32;
        let who = identity(me).expect("the test process is alive");
        assert_eq!(who.pid, me);
        assert!(who.start_time > 0);
        assert_eq!(identity(-1), None);
        assert_eq!(identity(2_000_000_000), None);
        assert_eq!(status_field(me, "NSpid"), me.to_string());
    }
}
