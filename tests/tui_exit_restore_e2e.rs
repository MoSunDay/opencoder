//! Process-level smoke: the TUI must restore the host terminal on EVERY exit
//! path — normal quit AND termination signals — so that after the process is
//! gone, mouse clicks/drags no longer print escape garbage into the shell
//! (the classic "left ?1000h mouse reporting on" brick).
//!
//! Runs the real binary under a real pty (via util-linux `script -q -f`),
//! waits for the mouse-capture enable sequence as the "terminal captured"
//! marker, then:
//!   1. SIGTERMs the opencoder process (not `script`) — the boot/onboarding
//!      window where the old signal handling was not yet armed is the exact
//!      regression this pins; the signal guard now arms at
//!      `TerminalGuard::enter()`.
//!   2. Sends Ctrl+D for the normal quit path.
//! Both must emit the full restoration payload (`?1000l…?1006l` mouse off,
//! `?2004l` paste off, `?1049l` leave alt-screen).
//!
//! Skips (not fails) when pty tooling is unavailable (non-Linux / minimal
//! images): `script`, `pgrep`, `/proc/<pid>/comm`.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_opencoder");

/// Kill-and-reap on drop so a failing assert never leaks processes.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn have(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// pid of the `opencoder` process whose cmdline contains `marker`
/// (distinguishes it from the `script`/`sh` wrappers and any unrelated
/// instances on the machine). None when it cannot be uniquely resolved.
fn find_opencoder_pid(marker: &str) -> Option<i32> {
    let out = Command::new("pgrep")
        .arg("-f")
        .arg(marker)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let pid: i32 = line.trim().parse().ok()?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        if comm.trim() == "opencoder" {
            return Some(pid);
        }
    }
    None
}

/// Spawn the TUI under a pty; returns (child, captured output, stdin handle).
fn spawn_tui(home: &std::path::Path, workdir: &std::path::Path) -> (Proc, Arc<Mutex<Vec<u8>>>, std::process::ChildStdin) {
    let cmd = format!("exec {BIN} tui --workdir {}", workdir.display());
    let mut child = Command::new("script")
        .args(["-q", "-f", "-c", &cmd, "/dev/null"])
        .env("HOME", home)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn script");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink.lock().unwrap().extend_from_slice(&buf[..n]),
            }
        }
    });
    (Proc(child), captured, stdin)
}

/// Block until the captured pty output contains `needle` (deadline-bounded).
fn wait_for(captured: &Arc<Mutex<Vec<u8>>>, needle: &[u8], what: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if captured.lock().unwrap().windows(needle.len()).any(|w| w == needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what} in pty output");
}

fn assert_restored(captured: &Arc<Mutex<Vec<u8>>>, ctx: &str) {
    let out = captured.lock().unwrap().clone();
    for (needle, what) in [
        (b"\x1b[?1000l" as &[u8], "disable mouse ?1000l"),
        (b"\x1b[?1006l", "disable mouse SGR ?1006l"),
        (b"\x1b[?2004l", "disable bracketed paste ?2004l"),
        (b"\x1b[?1049l", "leave alternate screen ?1049l"),
    ] {
        assert!(
            out.windows(needle.len()).any(|w| w == needle),
            "{ctx}: missing {what} — terminal left bricked (mouse/paste/alt-screen state would leak into the shell)"
        );
    }
}

#[test]
fn sigterm_after_capture_restores_terminal() {
    if !(have("script") && have("pgrep") && have("kill")) {
        eprintln!("skipping: script/pgrep/kill not available");
        return;
    }
    let home = tempfile::tempdir().expect("home tmp");
    let workdir = tempfile::tempdir().expect("workdir tmp");
    let (_proc, captured, _stdin) = spawn_tui(home.path(), workdir.path());

    // Marker: the TUI has captured the terminal (mouse reporting on). This is
    // the previously-unarmed window between TerminalGuard::enter() and the
    // liveness supervisor's spawn.
    wait_for(&captured, b"\x1b[?1000h", "mouse-capture enable");
    std::thread::sleep(Duration::from_millis(500)); // let the guard thread arm

    let marker = format!("{}", workdir.path().display());
    let pid = find_opencoder_pid(&marker)
        .expect("opencoder process not found under the pty wrapper");
    let ok = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("kill -TERM")
        .success();
    assert!(ok, "kill -TERM failed");

    wait_for(&captured, b"\x1b[?1049l", "restoration after SIGTERM");
    std::thread::sleep(Duration::from_millis(300));
    assert_restored(&captured, "SIGTERM exit");
    let binding = captured.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&binding);
    assert!(
        text.contains("terminal restored"),
        "SIGTERM exit must explain itself to the user: {text:?}"
    );
}

#[test]
fn normal_quit_restores_terminal() {
    if !(have("script") && have("pgrep")) {
        eprintln!("skipping: script/pgrep not available");
        return;
    }
    let home = tempfile::tempdir().expect("home tmp");
    let workdir = tempfile::tempdir().expect("workdir tmp");
    let (_proc, captured, mut stdin) = spawn_tui(home.path(), workdir.path());

    wait_for(&captured, b"\x1b[?1000h", "mouse-capture enable");
    std::thread::sleep(Duration::from_millis(800)); // let the first frame settle

    // Ctrl+D: quits both the onboarding wizard (fresh HOME) and the chat
    // composer (populated HOME) — whichever screen this machine reaches.
    stdin.write_all(b"\x04").expect("send Ctrl+D");

    wait_for(&captured, b"\x1b[?1049l", "restoration after quit");
    std::thread::sleep(Duration::from_millis(300));
    assert_restored(&captured, "normal quit");
}
