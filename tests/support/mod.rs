//! Shared helpers for the root-package fleet e2e tests: resolving the
//! workspace-sibling fleet binaries (`opencoder-server`, `opencoder-agent`)
//! from the same target dir as this package's own `opencoder` binary.

use std::path::PathBuf;

/// Candidate names for the fleet server binary, in priority order (see
/// [`sibling_bin`] for why there is more than one).
///
/// `allow(dead_code)`: every test file pulls in this whole module, and not
/// every file spawns both fleet binaries.
#[allow(dead_code)]
pub const SERVER_BIN: &[&str] = &["opencoder-server", "opencode-server"];

/// Candidate names for the fleet worker binary, in priority order.
#[allow(dead_code)]
pub const AGENT_BIN: &[&str] = &["opencoder-agent", "opencode-agent"];

/// Resolve a workspace-sibling binary from the same target dir as this
/// test binary, trying `candidates` in priority order and returning the
/// first one that exists.
///
/// Integration tests only get `CARGO_BIN_EXE_*` for targets of the package
/// that owns the test, but the fleet smokes deliberately live in the root
/// package while the server/agent binaries live in their own crates. The
/// workspace regression (`cargo test --workspace`) builds every member
/// binary into the same target dir before running any test, so resolving
/// siblings of this test's own binary is deterministic there.
///
/// Candidates (not a single name) because the fleet binaries carry the
/// package spelling (`opencoder-server`/`opencoder-agent`, matching the
/// `opencode daemon` migration hint) while some docs spell them without
/// the `r` (`opencode-server`/`opencode-agent`). Probing the package
/// spelling first with the documented one as fallback keeps these tests
/// green whichever way the naming settles.
pub fn sibling_bin(candidates: &[&str]) -> PathBuf {
    let own = PathBuf::from(env!("CARGO_BIN_EXE_opencoder"));
    let dir = own.parent().expect("test binary has a parent dir");
    for name in candidates {
        let path = dir.join(name);
        if path.is_file() {
            return path;
        }
    }
    panic!(
        "none of {candidates:?} found in {} — build the workspace binaries first: \
         `cargo build --workspace --bins` (fleet e2e smokes need the split \
         opencoder-server/opencoder-agent binaries; `cargo test -p opencoder` \
         alone does not build them)",
        dir.display()
    );
}
