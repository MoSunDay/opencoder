//! Shared helpers for the root-package fleet e2e tests: resolving the
//! workspace-sibling fleet binaries (`opencoder-server`, `opencoder-agent`)
//! from the same target dir as this package's own `opencoder` binary.

//! # Prerequisite
//!
//! The fleet smokes in this package spawn the split server/agent binaries,
//! which only exist in the shared target dir after a workspace build. Run
//! what [`FLEET_BINS_HINT`] says before running tests. `cargo test
//! --workspace` can build only a sibling's test harness and leave its
//! executable stale. Missing or mismatched siblings fail fast.

use std::path::PathBuf;

/// Remediation when no fleet sibling binary is found. A fixture-level
/// constant (not inline panic text) so the prerequisite stays single-source
/// and reusable by any future preflight.
pub const FLEET_BINS_HINT: &str = "build the workspace binaries first: \
     `cargo build --workspace --bins` (fleet e2e smokes need the split \
     opencoder-server/opencoder-agent binaries; `cargo test --workspace` \
     alone does not guarantee current sibling executables)";

/// Candidate names for the fleet server binary, in priority order (see
/// [`sibling_bin`] for why there is more than one).
///
/// `allow(dead_code)`: every test file pulls in this whole module, and not
/// every file spawns both fleet binaries.
#[allow(dead_code)]
pub const SERVER_BIN: &[&str] = &["opencoder-server", "opencoder-server"];

/// Candidate names for the fleet worker binary, in priority order.
#[allow(dead_code)]
pub const AGENT_BIN: &[&str] = &["opencoder-agent", "opencoder-agent"];

/// Candidate names for the control-plane CLI binary (DAG e2e drives the
/// real CLI face at least once per feature).
#[allow(dead_code)]
pub const CLI_BIN: &[&str] = &["opencoder-cli"];

/// Raw HTTP/SSE helpers shared by the e2e suites.
/// Each integration-test binary compiles this shared module separately and
/// exercises a different subset of its HTTP and lifecycle helpers.
#[allow(dead_code)]
pub mod http_util;

/// Real fleet process management (server + agent + loopback LLM stub).
/// Compiled separately per integration-test binary; not every target needs
/// every lifecycle operation or fixture path.
#[allow(dead_code)]
pub mod fleet_proc;

/// Deterministic OpenAI-compatible streaming stub.
/// Suites use different scripting modes, so some control methods are unused
/// within an individual integration-test binary.
#[allow(dead_code)]
pub mod llm_stub;

/// Resolve a workspace-sibling binary from the same target dir as this
/// test binary, trying `candidates` in priority order and returning the
/// first one that exists.
///
/// Integration tests only get `CARGO_BIN_EXE_*` for targets of the package
/// that owns the test, but the fleet smokes deliberately live in the root
/// package while the server/agent binaries live in their own crates. Verify
/// their compiled source metadata so cached executables cannot validate an
/// older revision while the test harness reports the current source.
///
/// Candidates (not a single name) because the fleet binaries carry the
/// package spelling (`opencoder-server`/`opencoder-agent`, matching the
/// `opencoder daemon` migration hint) while some docs spell them without
/// the `r` (`opencoder-server`/`opencoder-agent`). Probing the package
/// spelling first with the documented one as fallback keeps these tests
/// green whichever way the naming settles.
pub fn sibling_bin(candidates: &[&str]) -> PathBuf {
    let own = PathBuf::from(env!("CARGO_BIN_EXE_opencoder"));
    let dir = own.parent().expect("test binary has a parent dir");
    for name in candidates {
        let path = dir.join(name);
        if path.is_file() {
            let output = std::process::Command::new(&path)
                .arg("--build-info")
                .output()
                .expect(FLEET_BINS_HINT);
            assert!(output.status.success(), "{FLEET_BINS_HINT}");
            let actual: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect(FLEET_BINS_HINT);
            let expected = serde_json::to_value(opencoder_core::version::build_info()).unwrap();
            for key in ["git_commit", "git_dirty", "protocol_version"] {
                assert_eq!(
                    actual[key],
                    expected[key],
                    "{} has stale {key}; {FLEET_BINS_HINT}",
                    path.display()
                );
            }
            return path;
        }
    }
    panic!(
        "none of {candidates:?} found in {} — {FLEET_BINS_HINT}",
        dir.display()
    );
}
