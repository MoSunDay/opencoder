//! Build-time capture of git metadata.
//!
//! Resolves the short + full commit hash and the dirty flag at compile time and
//! exposes them (plus the pre-assembled long version string) to the crate via
//! `cargo:rustc-env`. Builds outside a git repo degrade gracefully to "unknown"
//! so compilation never fails for missing version info.
//!
//! `assemble` mirrors `core::version::format_version`; a unit test asserts the
//! baked string stays in lockstep with that pure helper (guards against drift).

use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    rerun_for_git(&manifest_dir);

    let short = run_git(&["rev-parse", "--short", "HEAD"], &manifest_dir)
        .unwrap_or_else(|| "unknown".to_string());
    let full =
        run_git(&["rev-parse", "HEAD"], &manifest_dir).unwrap_or_else(|| "unknown".to_string());
    let dirty = is_dirty(&manifest_dir);

    println!("cargo:rustc-env=OPENCODER_GIT_COMMIT={short}");
    println!("cargo:rustc-env=OPENCODER_GIT_COMMIT_FULL={full}");
    println!("cargo:rustc-env=OPENCODER_GIT_DIRTY={}", u8::from(dirty));

    let pkg = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    println!(
        "cargo:rustc-env=OPENCODER_VERSION_LONG={}",
        assemble(&pkg, &short, dirty)
    );
}

/// Re-run the build script when HEAD, loose refs, or packed refs move, so the
/// captured commit stays fresh across commits and branch switches. On a
/// normal branch HEAD only contains `ref: refs/heads/<name>` and does not
/// itself change when a new commit advances that ref.
fn rerun_for_git(manifest_dir: &str) {
    let git_dir = format!("{manifest_dir}/../../.git");
    println!("cargo:rerun-if-changed={git_dir}/HEAD");
    println!("cargo:rerun-if-changed={git_dir}/refs");
    println!("cargo:rerun-if-changed={git_dir}/packed-refs");
}

/// Run a git subcommand in `dir`, returning trimmed stdout on success.
fn run_git(args: &[&str], dir: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", dir])
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `git diff --quiet` / `--cached --quiet` exit success only when there are no
/// changes. Dirty when either the worktree or the index diverges from HEAD.
fn is_dirty(dir: &str) -> bool {
    let worktree_clean = git_exits_clean(&["diff", "--quiet"], dir);
    let index_clean = git_exits_clean(&["diff", "--cached", "--quiet"], dir);
    !(worktree_clean && index_clean)
}

fn git_exits_clean(args: &[&str], dir: &str) -> bool {
    Command::new("git")
        .args(["-C", dir])
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Assemble the long version string. Mirrors `core::version::format_version`.
fn assemble(version: &str, commit: &str, dirty: bool) -> String {
    if dirty {
        format!("{version} ({commit}-dirty)")
    } else {
        format!("{version} ({commit})")
    }
}
