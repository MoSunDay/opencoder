//! Source-contract: every production binary that hosts opencode sessions
//! must seed the built-in skill packs at startup.
//!
//! Skill assets are embedded in `opencoder-core` and resolved at session
//! time from the host's `~/.opencoder/skills`. A session host that skips
//! seeding leaves stale (or missing) installed copies behind after an
//! upgrade — the exact gap this contract locks shut. Two hosts exist: the
//! `opencoder` binary (headless `run`/`--cmd`, the TUI and `daemon` server
//! mode all dispatch from `src/main.rs`) and the `opencode-agent` worker
//! node (agent steps run the real session runner on the node's home).
//!
//! Source-level by design: startup wiring lives in `fn main`, which has no
//! callable seam; asserting on the source keeps the contract honest without
//! refactoring production entry points purely for testability.

const LOCAL_MAIN: &str = include_str!("../src/main.rs");
const AGENT_MAIN: &str = include_str!("../crates/agent/src/main.rs");

/// Both hosts must call the core seeding entry points — built-in packs
/// (update-on-drift) plus the sentinel-gated dep set. The exact call string
/// is required so a plain-text mention inside a comment cannot satisfy the
/// contract.
fn assert_seeds_at_startup(src: &str, label: &str) {
    assert!(
        src.contains("opencoder_core::seed_builtin_skills();"),
        "{label} must call opencoder_core::seed_builtin_skills() at startup"
    );
    assert!(
        src.contains("opencoder_core::seed_dep_gated_skills();"),
        "{label} must call opencoder_core::seed_dep_gated_skills() at startup"
    );
}

/// The seeding call must precede the first statement that can host a
/// session, so every session the binary runs resolves against freshly
/// seeded assets.
fn assert_seed_precedes_host(src: &str, label: &str, host_marker: &str) {
    let seed = src
        .find("opencoder_core::seed_builtin_skills();")
        .unwrap_or_else(|| panic!("{label}: seeding call missing"));
    let host = src
        .find(host_marker)
        .unwrap_or_else(|| panic!("{label}: host marker {host_marker:?} missing"));
    assert!(
        seed < host,
        "{label}: seeding must run before {host_marker:?} (first session host)"
    );
}

#[test]
fn local_binary_seeds_skills_before_any_dispatch() {
    assert_seeds_at_startup(LOCAL_MAIN, "opencoder (src/main.rs)");
    // `run_headless` is the earliest dispatch arm reached for `--cmd` and
    // `run`; the TUI, ts and daemon arms dispatch later in the same match.
    assert_seed_precedes_host(LOCAL_MAIN, "opencoder (src/main.rs)", "run_headless");
}

#[test]
fn agent_binary_seeds_skills_before_worker_opens() {
    assert_seeds_at_startup(AGENT_MAIN, "opencode-agent (crates/agent/src/main.rs)");
    // Worker::open boots the node runtime that executes agent sessions.
    assert_seed_precedes_host(
        AGENT_MAIN,
        "opencode-agent (crates/agent/src/main.rs)",
        "Worker::open",
    );
}
