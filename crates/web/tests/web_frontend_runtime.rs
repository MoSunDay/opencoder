//! Runtime acceptance for the embedded web frontend (the piece a browser
//! walkthrough would cover, automated): `frontend_smoke.mjs` loads the REAL
//! asset scripts under node with a DOM shim + mock fetch and asserts the
//! interactive behaviors static html.rs tests cannot see — question cards,
//! queue panel list/reorder/delete, model dropdown, SSE reconnect badge,
//! composer send. Skips (pass, with a note) when node is not installed.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn frontend_headless_smoke() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/frontend_smoke.mjs");
    assert!(
        script.exists(),
        "smoke script missing: {}",
        script.display()
    );
    let node = std::env::var("NODE_BIN").unwrap_or_else(|_| "node".to_string());
    let out = match Command::new(&node).arg(&script).output() {
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
        "frontend headless smoke failed:\n{stdout}\n{stderr}"
    );
}
