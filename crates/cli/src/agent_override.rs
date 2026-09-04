//! `--agent` override plumbing for the headless run path. Extracted from
//! `run.rs` (which keeps the flow) so that file stays under the iteration
//! line cap; mirrors `model_override.rs` for the agent side.
//!
//! Resolution goes through `opencoder_core::resolve_agent`, so file-based
//! custom agents (`~/.opencoder/agents/<name>/meta.json`) are accepted
//! everywhere a builtin is — parse time, fresh sessions, and resume
//! re-application.

use anyhow::{anyhow, Result};
use opencoder_core::{resolve_agent, Config};
use opencoder_session::SessionState;

/// Apply an `--agent` override (builtin name like plan/explore/build) to the
/// config. Sets `config.agent.default` so the fresh-session path resolves it.
/// Returns true when the config changed.
pub(crate) fn apply_agent_override(config: &mut Config, agent: &Option<String>) -> bool {
    if let Some(a) = agent {
        if config.agent.default != *a {
            config.agent.default = a.clone();
            return true;
        }
    }
    false
}

/// Re-apply an explicit `--agent` to a resumed session. `resume()` restores the
/// stored agent into the session, so an explicit `--agent` must win here. Returns
/// the new agent name when the session was changed (caller persists it), else None.
pub(crate) fn reapply_resume_agent(
    session: &mut SessionState,
    agent: &Option<String>,
) -> Result<Option<String>> {
    let name = match agent.as_ref() {
        Some(n) => n,
        None => return Ok(None),
    };
    if session.agent.name == *name {
        return Ok(None);
    }
    // `name` here is always an explicit --agent value (we returned early on
    // None), so an unknown name must error rather than silently resolve to
    // "act".
    let resolved = resolve_agent(name).ok_or_else(|| anyhow!("agent not found: {name}"))?;
    session.agent = resolved;
    Ok(Some(name.clone()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Serializes tests that touch the process-global agents-root override.
    /// Shared crate-wide (the override is process-global, so per-module locks
    /// would still race — same pattern as `core::agent::meta::tests`).
    pub(crate) static OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Minimal resolvable file agent: a private prompt pool
    /// `prompts/<name>/v1` (soul only) plus a card referencing it.
    pub(crate) fn write_file_agent(root: &std::path::Path, name: &str, soul: &str) {
        let pool = root.join("prompts").join(name);
        let vdir = pool.join("v1");
        std::fs::create_dir_all(&vdir).unwrap();
        std::fs::write(vdir.join("soul.md"), soul).unwrap();
        std::fs::write(
            pool.join("meta.json"),
            format!(r#"{{ "name": "{name}", "current": 1, "history": [1] }}"#),
        )
        .unwrap();
        let adir = root.join(name);
        std::fs::create_dir_all(&adir).unwrap();
        std::fs::write(
            adir.join("meta.json"),
            format!(r#"{{ "name": "{name}", "current": {{ "prompt": "{name}" }} }}"#),
        )
        .unwrap();
    }

    #[test]
    fn apply_agent_override_sets_default() {
        let mut cfg = Config::default();
        assert_eq!(cfg.agent.default, "act");
        assert!(apply_agent_override(&mut cfg, &Some("plan".into())));
        assert_eq!(cfg.agent.default, "plan");
        // same value -> no change (returns false)
        assert!(!apply_agent_override(&mut cfg, &Some("plan".into())));
        // no override -> no change
        let mut cfg2 = Config::default();
        let before = cfg2.agent.default.clone();
        assert!(!apply_agent_override(&mut cfg2, &None));
        assert_eq!(cfg2.agent.default, before);
    }

    #[test]
    fn reapply_resume_agent_overrides_stored_agent() {
        use opencoder_llm::{ChatStream, MockChatClient};
        use std::sync::Arc;
        // simulate a session resumed with the default "act" agent
        let cfg = Config::default();
        let agent = resolve_agent("act").unwrap();
        let mut s = SessionState::new(
            "s1",
            agent,
            cfg,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        // explicit --agent plan wins over the resumed "act"
        let changed = reapply_resume_agent(&mut s, &Some("plan".into())).unwrap();
        assert_eq!(changed.as_deref(), Some("plan"));
        assert_eq!(s.agent.name, "plan");
        // same value -> no change, returns None
        assert_eq!(
            reapply_resume_agent(&mut s, &Some("plan".into())).unwrap(),
            None
        );
        // no override -> no change, returns None
        assert_eq!(reapply_resume_agent(&mut s, &None).unwrap(), None);
    }

    #[test]
    fn reapply_resume_agent_rejects_unknown_name() {
        use opencoder_llm::{ChatStream, MockChatClient};
        use std::sync::Arc;
        // A typo'd/explicit-but-unknown agent name must error rather than
        // silently resolving to "act".
        let cfg = Config::default();
        let agent = resolve_agent("act").unwrap();
        let mut s = SessionState::new(
            "s1",
            agent,
            cfg,
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        let err = reapply_resume_agent(&mut s, &Some("nonexistent-agent".into())).unwrap_err();
        assert!(
            err.to_string()
                .contains("agent not found: nonexistent-agent"),
            "expected unknown-agent error, got: {err}"
        );
        // The sandbox-mode interlude: "sandbox" is no longer a resolvable
        // builtin, so an explicit --agent sandbox must be rejected too.
        let err = reapply_resume_agent(&mut s, &Some("sandbox".into())).unwrap_err();
        assert!(
            err.to_string().contains("agent not found: sandbox"),
            "expected removed-sandbox-agent error, got: {err}"
        );
        // session agent unchanged by the failed reapply
        assert_eq!(s.agent.name, "act");
    }

    /// File agents flow through both seams: `--agent <file-agent>` on a
    /// resumed session swaps to it, and the fresh-session chain run.rs uses
    /// (fold into config -> `effective_default_agent` -> `resolve_agent`)
    /// lands on it.
    #[test]
    fn file_agent_flows_through_both_seams() {
        use opencoder_core::agent::{effective_default_agent, set_agents_dir_override};

        let dir = tempfile::tempdir().unwrap();
        let _g = OVERRIDE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_agents_dir_override(Some(dir.path().to_path_buf()));
        write_file_agent(dir.path(), "writer", "Writer soul: small, focused diffs.");

        // Resume seam: explicit --agent writer wins over the stored "act".
        use opencoder_llm::{ChatStream, MockChatClient};
        use std::sync::Arc;
        let mut s = SessionState::new(
            "s1",
            resolve_agent("act").unwrap(),
            Config::default(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::path::PathBuf::from("/tmp"),
        );
        let changed = reapply_resume_agent(&mut s, &Some("writer".into())).unwrap();
        assert_eq!(changed.as_deref(), Some("writer"));
        assert_eq!(s.agent.name, "writer");
        assert!(s.agent.is_primary());
        assert!(
            s.agent.prompt.contains("Writer soul"),
            "resolved file agent must carry the card's prompt body"
        );

        // Fresh-session chain: fold -> effective default -> resolve.
        let mut cfg = Config::default();
        assert!(apply_agent_override(&mut cfg, &Some("writer".into())));
        let name = effective_default_agent(None, &cfg);
        assert_eq!(name, "writer");
        let agent = resolve_agent(&name).unwrap();
        assert_eq!(agent.name, "writer");
        assert!(agent.prompt.contains("Writer soul"));
    }
}
