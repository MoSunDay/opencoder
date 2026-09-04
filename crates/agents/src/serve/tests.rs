//! Unit tests for the NFS server contract: spawn/shutdown lifecycle,
//! status shape, config plumbing, and a manual (ignored) real-mount e2e.
use super::*;
use crate::testutil;
use opencoder_core::config::Config;

fn opts(root: &Path, port: u16) -> NfsServerOpts {
    NfsServerOpts {
        export_root: root.to_path_buf(),
        host: "127.0.0.1".to_string(),
        port,
        read_only: true,
    }
}

/// Smoke: ephemeral bind is reachable over plain TCP, status reports
/// the resolved shape, shutdown returns promptly and releases the port
/// (a second spawn on the same port succeeds). No NFS handshake —
/// that's covered by the manual `mount` e2e below.
#[tokio::test]
async fn smoke_bind_status_shutdown_port_released() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.json"), b"{}").unwrap();
    let h = spawn_nfs_server(&opts(dir.path(), 0)).unwrap();

    let addr = h.local_addr().unwrap();
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert_ne!(addr.port(), 0);

    let st = nfs_status(Some(&h));
    assert!(st.running);
    assert_eq!(st.host, "127.0.0.1");
    assert_eq!(st.port, addr.port());
    assert!(st.read_only);
    assert_eq!(
        st.export_root,
        dir.path().canonicalize().unwrap().display().to_string()
    );
    assert_eq!(h.export_root(), dir.path().canonicalize().unwrap());
    assert!(h.read_only());

    let started = Instant::now();
    h.shutdown();
    // Bounded: comfortably under the deadline even with the helper
    // thread's final 5ms sleep on a loaded CI box.
    assert!(
        started.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_secs(1),
        "shutdown must be bounded"
    );

    let h2 = spawn_nfs_server(&opts(dir.path(), addr.port())).unwrap();
    assert_eq!(h2.local_addr().unwrap().port(), addr.port());
    h2.shutdown();
}

/// Bad export roots fail at spawn, with context.
#[tokio::test]
async fn spawn_rejects_missing_or_non_dir_root() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert!(spawn_nfs_server(&opts(&missing, 0)).is_err());
    let file = dir.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    assert!(spawn_nfs_server(&opts(&file, 0)).is_err());
}

/// A port already bound by the first server is refused for the second.
#[tokio::test]
async fn spawn_conflict_surfaces_bind_error() {
    let dir = tempfile::tempdir().unwrap();
    let a = spawn_nfs_server(&opts(dir.path(), 0)).unwrap();
    let port = a.local_addr().unwrap().port();
    assert!(spawn_nfs_server(&opts(dir.path(), port)).is_err());
    a.shutdown();
}

/// Works from a plain thread with no ambient tokio runtime (dedicated
/// runtime thread), and the resulting listener answers TCP.
#[test]
fn spawn_works_without_ambient_runtime() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("prompts")).unwrap();
    let h = spawn_nfs_server(&opts(dir.path(), 0)).unwrap();
    let addr = h.local_addr().unwrap();
    let sock = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    drop(sock);
    h.shutdown();
}

/// `None` ⇒ documented stopped defaults; `Some` mirrors the handle.
#[test]
fn status_shape() {
    let st = nfs_status(None);
    assert!(!st.running);
    assert_eq!(st.host, "127.0.0.1");
    assert_eq!(st.port, 2049);
    assert!(st.read_only);
    assert_eq!(st.export_root, "");
}

/// Defaults round-trip from a default `Config` under the override lock
/// (`agents_dir()` reads process-global state). The lock is released
/// between the `scoped()` blocks — it is not reentrant.
#[test]
fn default_opts_from_config_roundtrip() {
    let o = default_opts_from_config(&Config::default());
    assert_eq!(o.host, "127.0.0.1");
    assert_eq!(o.port, 2049);
    assert!(o.read_only);

    {
        let (_dir, guard) = testutil::scoped();
        let o = default_opts_from_config(&Config::default());
        assert_eq!(o.export_root, opencoder_core::agent::agents_dir().unwrap());
        drop(guard);
    }

    // An explicit agents_dir wins over the resolution chain.
    {
        let (dir, guard) = testutil::scoped();
        let custom = dir.path().join("custom-agents");
        let cfg = Config {
            agent: opencoder_core::config::AgentDefaults {
                agents_dir: Some(custom.clone()),
                ..Config::default().agent
            },
            ..Config::default()
        };
        let o = default_opts_from_config(&cfg);
        assert_eq!(o.export_root, custom);
        drop(guard);
    }

    let raw = r#"{ "agent": { "nfs": { "enabled": true, "host": "0.0.0.0", "port": 3050, "read_only": false } } }"#;
    let cfg: Config = serde_json::from_str(raw).unwrap();
    let o = default_opts_from_config(&cfg);
    assert_eq!(o.host, "0.0.0.0");
    assert_eq!(o.port, 3050);
    assert!(!o.read_only);
}

/// Real-client e2e: mount the export with `mount.nfs` and read the
/// tree through the kernel. Ignored by default — it needs
/// root/CAP_SYS_ADMIN, an installed `mount.nfs` and the nfs client
/// modules. Run with `cargo test -p opencode-agents -- --ignored`.
#[test]
#[ignore = "manual: needs mount privileges + nfs client"]
fn manual_mount_e2e() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("meta.json"), br#"{"name":"p"}"#).unwrap();
    let v1 = dir.path().join("prompts/p/v1");
    std::fs::create_dir_all(&v1).unwrap();
    std::fs::write(v1.join("soul.md"), b"be terse").unwrap();
    let mnt = tempfile::tempdir().unwrap();

    let h = spawn_nfs_server(&opts(dir.path(), 0)).unwrap();
    let port = h.local_addr().unwrap().port();
    let mount_opts =
        format!("vers=3,tcp,port={port},mountport={port},nolock,soft,retrans=1,timeo=50");
    let out = Command::new("mount")
        .args(["-t", "nfs", "-o", &mount_opts, "127.0.0.1:/"])
        .arg(mnt.path())
        .output()
        .expect("mount binary present");
    assert!(
        out.status.success(),
        "mount failed: {}\n{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let mounted = || {
        Command::new("mount")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(mnt.path().to_str().unwrap()))
    };
    assert!(
        mounted().unwrap_or(false),
        "mountpoint not listed in /proc/mounts-equivalent"
    );
    // Reads flow through the full NFS stack.
    assert_eq!(
        std::fs::read(mnt.path().join("prompts/p/v1/soul.md")).unwrap(),
        b"be terse"
    );
    assert!(std::fs::read_to_string(mnt.path().join("meta.json"))
        .unwrap()
        .contains("p"));
    // Writes are rejected read-only.
    assert!(std::fs::write(mnt.path().join("meta.json"), b"x").is_err());
    // Cleanup before the server goes away.
    let _ = Command::new("umount").arg(mnt.path()).output();
    h.shutdown();
}
