//! Process-level wrapper that wires scripts/smoke_nodes.sh into the cargo
//! test path (`cargo test --test nodes_smoke_proc`), so the two-process
//! distributed-nodes smoke no longer depends on a human remembering to run
//! a shell script outside CI. The script itself stays the single source of
//! truth for assertions: this wrapper only injects the freshly built debug
//! binaries (`OPENCODER_SMOKE_SERVER_BIN`/`OPENCODER_SMOKE_AGENT_BIN`, the
//! split fleet pair) and a random port
//! (`OPENCODER_SMOKE_PORT`, avoids colliding with parallel tests), enforces
//! an outer watchdog timeout (the script's internal curl polls never time
//! out on their own), and verifies the success marker. Checkpoint 3 in the
//! script allows `error` as terminal task state, so the script's seeded
//! loopback LLM stub keeps the run deterministic with zero credentials.

mod support;

use std::io::Read;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hard ceiling for the whole script run (build is skipped via the injected
/// binaries; server+node startup and the 4 checkpoints fit well inside this).
const TIMEOUT_SECS: u64 = 300;
/// Success marker printed by scripts/smoke_nodes.sh after all checkpoints.
const PASSED_MARKER: &str = "SMOKE NODES PASSED";
/// Port range for injection — mirrors other smokes' ephemeral-port hygiene.
const PORT_MIN: u16 = 18000;
const PORT_MAX: u16 = 19000;

/// Pick a port inside [PORT_MIN, PORT_MAX] that is free right now, seeded
/// from pid + wall clock so parallel cargo test targets rarely collide.
fn pick_port() -> u16 {
    let span = u32::from(PORT_MAX - PORT_MIN);
    let seed = std::process::id() as u32
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
    for i in 0..64u32 {
        let candidate = PORT_MIN
            + ((seed.wrapping_mul(2654435761).rotate_left(7).wrapping_add(i)) % span) as u16;
        if TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate; // dropped immediately: tiny race, but worst case the script fails loudly
        }
    }
    PORT_MIN + (seed % span) as u16
}

fn tail(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

/// Kill the whole process group spawned around `child` (the script forks its
/// own server/node children; killing bash alone would orphan them holding
/// the injected port).
fn kill_tree(child: &mut std::process::Child) {
    let neg_pid = format!("-{}", child.id());
    let _ = Command::new("kill")
        .arg("-9")
        .arg("--")
        .arg(&neg_pid)
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn smoke_script_two_process_nodes_flow_passes() {
    let repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = repo_root.join("scripts").join("smoke_nodes.sh");
    assert!(
        script.is_file(),
        "smoke script missing at {}",
        script.display()
    );

    let mut child = Command::new("bash")
        .arg(&script)
        .current_dir(&repo_root)
        .env(
            "OPENCODER_SMOKE_SERVER_BIN",
            support::sibling_bin(support::SERVER_BIN),
        )
        .env(
            "OPENCODER_SMOKE_AGENT_BIN",
            support::sibling_bin(support::AGENT_BIN),
        )
        .env("OPENCODER_SMOKE_PORT", pick_port().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bash scripts/smoke_nodes.sh");

    let deadline = Instant::now() + Duration::from_secs(TIMEOUT_SECS);
    loop {
        match child.try_wait().expect("try_wait smoke script") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut p) = child.stdout.take() {
                    let _ = p.read_to_string(&mut stdout);
                }
                if let Some(mut p) = child.stderr.take() {
                    let _ = p.read_to_string(&mut stderr);
                }
                assert!(
                    status.success(),
                    "scripts/smoke_nodes.sh exited with {status}\n--- stdout tail ---\n{}\n--- stderr tail ---\n{}",
                    tail(&stdout, 60),
                    tail(&stderr, 40)
                );
                assert!(
                    stdout.contains(PASSED_MARKER),
                    "script succeeded but never printed `{PASSED_MARKER}`\n--- stdout tail ---\n{}",
                    tail(&stdout, 60)
                );
                return;
            }
            None if Instant::now() > deadline => {
                kill_tree(&mut child);
                panic!(
                    "scripts/smoke_nodes.sh exceeded the {TIMEOUT_SECS}s watchdog; \
                     process tree killed (a checkpoint likely hangs — check node/server logs)"
                );
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
