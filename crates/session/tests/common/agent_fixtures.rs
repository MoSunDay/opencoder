//! File-based-agent fixtures for the session integration suites.
//!
//! Hand-builds the on-disk layout the core read path expects under the
//! agents root — a thin reference card (`<agent>/meta.json`) plus shared,
//! versioned pools (`prompts|skills|tools/<name>/v{n}/…`) — behind a
//! per-binary lock that also isolates `$HOME`, so the global skills dir
//! and the agents root are fully test-controlled while a fixture is live.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use opencoder_core::agent::meta::set_agents_dir_override;

/// Serializes every fixture flip inside ONE test binary: the agents-root
/// override and `$HOME` are process-global, so concurrent tests in this
/// binary must not observe another test's roots.
pub static AGENT_FIXTURE_LOCK: Mutex<()> = Mutex::new(());

/// Restores `$HOME` + the agents-root override when dropped.
pub struct AgentFixtureGuard {
    prev_home: Option<std::ffi::OsString>,
    pub home: PathBuf,
    pub agents: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

/// Point the process at fresh fixture roots: `agents` becomes the
/// agents root (override) and `home` (with `home/.opencoder/skills` for
/// global-skill fixtures) becomes `$HOME`. Hold the returned guard for
/// the whole test body.
pub fn scoped_agent_roots() -> AgentFixtureGuard {
    let lock = AGENT_FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let agents = tempfile::tempdir().expect("agents root tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    std::fs::create_dir_all(home.path().join(".opencoder/skills")).expect("skills dir");
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("HOME", home.path());
    set_agents_dir_override(Some(agents.path().to_path_buf()));
    AgentFixtureGuard {
        prev_home,
        home: home.path().to_path_buf(),
        agents: agents.path().to_path_buf(),
        _lock: lock,
    }
}

impl Drop for AgentFixtureGuard {
    fn drop(&mut self) {
        set_agents_dir_override(None);
        match self.prev_home.take() {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// Write one pool version: `<cat>/<name>/v<n>/<rel>` plus the pool meta
/// pinning `current` to the highest written version.
pub fn write_pool_version(root: &Path, cat: &str, name: &str, version: u32, rel: &str, body: &str) {
    let dir = root.join(cat).join(name).join(format!("v{version}"));
    std::fs::create_dir_all(dir.join(rel).parent().unwrap_or(&dir)).expect("pool version dir");
    std::fs::write(dir.join(rel), body).expect("pool version file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if rel.ends_with(".sh") || !rel.contains('.') {
            let _ = std::fs::set_permissions(dir.join(rel), std::fs::Permissions::from_mode(0o755));
        }
    }
    let pool_dir = root.join(cat).join(name);
    std::fs::write(
        pool_dir.join("meta.json"),
        format!("{{\"name\": \"{name}\", \"current\": {version}, \"history\": [{version}]}}"),
    )
    .expect("pool meta");
}

/// Make one file executable (unix only; a no-op elsewhere).
pub fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Write an agent reference card referencing the given pools by name.
pub fn write_agent_card(root: &Path, name: &str, prompt: Option<&str>, skills: Option<&str>, tools: Option<&str>) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("agent dir");
    let opt = |v: Option<&str>| match v {
        Some(s) => format!("\"{s}\""),
        None => "null".to_string(),
    };
    let meta = format!(
        "{{\"name\": \"{name}\", \"current\": {{\"prompt\": {}, \"skills\": {}, \"tools\": {}}}}}",
        opt(prompt),
        opt(skills),
        opt(tools)
    );
    std::fs::write(dir.join("meta.json"), meta).expect("agent meta");
}

/// A complete file agent: prompt pool (soul body `SOUL-<name>`), optional
/// skills pool carrying one `alpha` skill, optional tools pool carrying an
/// executable `probe-tool` printing `PROBE-<version>`.
pub fn write_full_agent(root: &Path, name: &str, with_skills: bool, with_tools: bool, tool_version: u32) {
    write_pool_version(root, "prompts", name, 1, "soul.md", &format!("SOUL-{name} identity"));
    if with_skills {
        write_pool_version(
            root,
            "skills",
            "alpha-set",
            1,
            "alpha/SKILL.md",
            "AGENT-ALPHA body",
        );
    }
    if with_tools {
        let v = tool_version;
        write_pool_version(
            root,
            "tools",
            "t",
            v,
            "probe-tool",
            &format!("#!/bin/sh\necho PROBE-v{v}\n"),
        );
        make_executable(&root.join("tools/t").join(format!("v{v}")).join("probe-tool"));
    }
    write_agent_card(
        root,
        name,
        Some(name),
        with_skills.then_some("alpha-set"),
        with_tools.then_some("t"),
    );
}

/// Plant a global skill under the fixture `$HOME` (`<name>/SKILL.md`).
pub fn write_global_skill(home: &Path, name: &str, body: &str) {
    let dir = home.join(".opencoder/skills").join(name);
    std::fs::create_dir_all(&dir).expect("global skill dir");
    std::fs::write(dir.join("SKILL.md"), body).expect("global SKILL.md");
}
