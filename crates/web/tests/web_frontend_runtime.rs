//! Runtime acceptance for the embedded web frontend (the piece a browser
//! walkthrough would cover, automated): `frontend_smoke.mjs` and
//! `frontend_nodes.mjs` load the REAL asset scripts under node with the
//! shared `dom_shim.mjs` (DOM shim + mock fetch + EventSource stub) and
//! assert the interactive behaviors static html.rs tests cannot see —
//! question cards, queue panel, composer send, SSE reconnect badge, and the
//! Phase-4 nodes panel (registry, dispatch, live task stream, cancel).
//! Skips (pass, with a note) when node is not installed.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn frontend_headless_smoke() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    for script in ["frontend_smoke.mjs", "frontend_nodes.mjs"] {
        let path = dir.join(script);
        assert!(path.exists(), "smoke script missing: {}", path.display());
        let node = std::env::var("NODE_BIN").unwrap_or_else(|_| "node".to_string());
        let out = match Command::new(&node).arg(&path).output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipping frontend smoke: node not found (set NODE_BIN to override)");
                return;
            }
            Err(e) => panic!("failed to spawn {node}: {e}"),
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        print!("{stdout}");
        assert!(
            out.status.success(),
            "frontend headless smoke failed ({script}):\n{stdout}\n{stderr}"
        );
    }
}
