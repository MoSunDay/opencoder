//! Session-side glue for file-based agent resource pools: the agent-private
//! tools PATH dirs and skill roots that follow the session's CURRENT agent.
//!
//! Pure functions over (`Config`, agent name) — no state lives here.
//! [`SessionState`] snapshots both surfaces into `tools_path`/`skill_roots`
//! and every site that changes the session agent (construction, resume,
//! `/agent` + mode switches, config reload) re-derives them via [`refresh`].
//! The skill choke points in the live session (`skill_resolve`, autopilot)
//! discover through [`discover_session_skills`] so an `/agent`-switched
//! session uses ITS agent's skill pools, shadowing same-name global skills
//! (first-wins, `discover_all`). TUI/store/cli callers of the plain marker-
//! based `skill::discover()` are untouched by design.

use std::path::PathBuf;

use opencoder_core::config::Config;
use opencoder_core::skill::{discover_cached, skills_dir, Skill};

use crate::SessionState;

/// Tool dirs a session exposes for `agent_name`, honoring the config scope
/// (`All` = every registered tools pool, `Active` = the named agent's
/// `current.tools` ref). Builtin agents without a file card yield empty.
pub fn tools_path_for(cfg: &Config, agent_name: &str) -> Vec<PathBuf> {
    opencoder_core::agent::tools_paths(cfg.agent.tools_scope, Some(agent_name))
}

/// Skill-pool roots of `agent_name` (its `current.skills` ref's current
/// version dir; 0–1 entries, empty for builtins without a file card).
pub fn skill_roots_for(agent_name: &str) -> Vec<PathBuf> {
    opencoder_core::agent::meta::agent_skill_roots(agent_name)
}

/// Ordered skill-discovery roots for `session`: the agent's private pools
/// FIRST, then the global skills dir. Earlier roots shadow later ones.
pub fn session_skill_roots(session: &SessionState) -> Vec<PathBuf> {
    let mut roots = session.skill_roots.clone();
    if let Some(dir) = skills_dir() {
        roots.push(dir);
    }
    roots
}

/// Discover the live session's skills through its agent-aware root list
/// (cached like the global `discover()` path). This is the choke-point
/// helper: a `/agent`-switched session resolves skill tokens against its
/// own agent's pools before the global dir.
pub fn discover_session_skills(session: &SessionState) -> Vec<Skill> {
    discover_cached(&session_skill_roots(session))
}

/// Refresh both pool snapshots on `session` from its CURRENT agent and
/// config. Called wherever the session agent can change (construction,
/// resume, `/agent`/`/act`/`/plan` switches, config reload — the tools
/// scope may flip) so the snapshots never go stale.
pub fn refresh(session: &mut SessionState) {
    session.tools_path = tools_path_for(&session.config, &session.agent.name);
    session.skill_roots = skill_roots_for(&session.agent.name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::{Arc, MutexGuard};

    use opencoder_core::agent::meta::set_agents_dir_override;
    use opencoder_core::{resolve_agent, Config};
    use opencoder_llm::{ChatStream, MockChatClient};

    /// Restores `$HOME` and the process-global agents-root override on drop,
    /// under the shared env lock so concurrent env-flipping tests serialize.
    struct RootsGuard {
        prev_home: Option<std::ffi::OsString>,
        _home: tempfile::TempDir,
        _env: MutexGuard<'static, ()>,
    }

    /// Point the agents root at a fresh tempdir, isolated from the ambient
    /// `$HOME`-derived global skills dir. The returned guard restores both.
    /// Holds the shared env lock for its whole lifetime so every unit test
    /// that flips the process-global agents root serializes against the
    /// other env-flipping suites.
    fn scoped_roots() -> (tempfile::TempDir, RootsGuard) {
        let _env = crate::test_env::env_lock();
        let agents = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());
        set_agents_dir_override(Some(agents.path().to_path_buf()));
        let guard = RootsGuard {
            prev_home,
            _home: home,
            _env,
        };
        (agents, guard)
    }

    impl Drop for RootsGuard {
        fn drop(&mut self) {
            set_agents_dir_override(None);
            match self.prev_home.take() {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    fn session_for(agent: &str) -> SessionState {
        SessionState::new(
            "pools-test",
            resolve_agent(agent).unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::env::temp_dir(),
        )
    }

    /// Card + pools fixture for one file agent: prompt pool, skills pool,
    /// tools pool, all referenced by the card.
    fn write_agent(root: &Path, name: &str, skills_ref: Option<&str>, tools_ref: Option<&str>) {
        write_pool(root, "prompts", name, "soul.md", "soul");
        write_pool(
            root,
            "skills",
            "alpha-set",
            "alpha/SKILL.md",
            "agent alpha body",
        );
        write_pool(root, "tools", "t", "probe-tool", "#!/bin/sh\necho probe\n");
        let refs = format!(
            "{{\"current\": {{\"prompt\": \"{name}\", \"skills\": {sj}, \"tools\": {tj}}}}}",
            sj = skills_ref
                .map(|s| format!("\"{s}\""))
                .unwrap_or("null".into()),
            tj = tools_ref
                .map(|s| format!("\"{s}\""))
                .unwrap_or("null".into()),
        );
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("meta.json"), refs).unwrap();
    }

    fn write_pool(root: &Path, cat: &str, name: &str, rel: &str, body: &str) {
        let dir = root.join(cat).join(name).join("v1");
        std::fs::create_dir_all(dir.join(rel).parent().unwrap()).unwrap();
        std::fs::write(dir.join(rel), body).unwrap();
        std::fs::write(
            root.join(cat).join(name).join("meta.json"),
            format!("{{\"name\": \"{name}\", \"current\": 1, \"history\": [1]}}"),
        )
        .unwrap();
    }

    #[test]
    fn builtin_sessions_carry_empty_pools() {
        let (_a, _g) = scoped_roots();
        let s = session_for("act");
        assert!(s.tools_path.is_empty());
        assert!(s.skill_roots.is_empty());
    }

    #[test]
    fn file_agent_session_snapshots_both_pools() {
        let (agents, _g) = scoped_roots();
        write_agent(agents.path(), "worker", Some("alpha-set"), Some("t"));

        let s = session_for("worker");
        assert_eq!(
            s.tools_path,
            vec![agents.path().join("tools/t/v1")],
            "tools snapshot follows the agent's tools ref"
        );
        assert_eq!(
            s.skill_roots,
            vec![agents.path().join("skills/alpha-set/v1")],
            "skill-roots snapshot follows the agent's skills ref"
        );
        // Choke-point discovery sees the agent pool's skill.
        let skills = discover_session_skills(&s);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[0].body, "agent alpha body");
    }

    #[test]
    fn refresh_follows_agent_and_scope_changes() {
        let (agents, _g) = scoped_roots();
        write_agent(agents.path(), "worker", Some("alpha-set"), Some("t"));

        let mut s = session_for("act");
        assert!(s.tools_path.is_empty() && s.skill_roots.is_empty());
        s.agent = resolve_agent("worker").unwrap();
        refresh(&mut s);
        assert_eq!(s.tools_path, vec![agents.path().join("tools/t/v1")]);
        assert_eq!(
            s.skill_roots,
            vec![agents.path().join("skills/alpha-set/v1")]
        );

        // Scope flip via config reload also flows through refresh.
        let mut cfg_all = Config::default();
        cfg_all.agent.tools_scope = opencoder_core::config::ToolsScope::All;
        s.apply_config_reload_keep_client(cfg_all);
        assert_eq!(s.tools_path, vec![agents.path().join("tools/t/v1")]);
    }
}
