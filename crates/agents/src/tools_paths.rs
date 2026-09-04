//! Tool-surface paths for custom agents — pure delegation to the core
//! read path so session/web never touch the agents-tree layout.

use std::path::PathBuf;

use opencoder_core::agent::{active_tools_dirs, agent_tools_dirs, all_tools_dirs};
use opencoder_core::config::ToolsScope;

/// Resolve the tool directories a session should expose:
///
/// - `All` → current version dirs of **every** tools resource (union
///   surface);
/// - `Active` + explicit agent name → that agent's `current.tools` ref;
/// - `Active` + `None` → the active agent's (empty when no marker).
pub fn tools_paths(scope: ToolsScope, agent: Option<&str>) -> Vec<PathBuf> {
    match scope {
        ToolsScope::All => all_tools_dirs(),
        ToolsScope::Active => match agent {
            Some(name) => agent_tools_dirs(name),
            None => active_tools_dirs(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scoped;
    use crate::write::{create_agent, save_resource_version, VersionFile};
    use opencoder_core::agent::{read_resource_meta, set_active_agent, AgentRefs};

    fn kit(name: &str, file: &str) {
        save_resource_version(
            "tools",
            name,
            &[VersionFile {
                rel_path: file.into(),
                bytes: b"x".to_vec(),
            }],
        )
        .unwrap();
    }

    #[test]
    fn tools_paths_covers_all_three_scopes() {
        let (tmp, _g) = scoped();
        kit("a", "run-a.sh");
        kit("b", "run-b.sh");
        create_agent(
            "worker",
            AgentRefs {
                tools: Some("b".into()),
                ..Default::default()
            },
        )
        .unwrap();
        set_active_agent(Some("worker")).unwrap();

        // All → union of every tools resource's current version dir.
        let all = tools_paths(ToolsScope::All, None);
        assert_eq!(all.len(), 2);
        // Active + explicit name → that agent's tools ref.
        let named = tools_paths(ToolsScope::Active, Some("worker"));
        assert_eq!(named, vec![tmp.path().join("tools/b/v1")]);
        // Active + None → the active agent's tools ref.
        assert_eq!(tools_paths(ToolsScope::Active, None), named);
        // An agent with no tools ref resolves to the empty surface.
        create_agent("bare", Default::default()).unwrap();
        assert!(tools_paths(ToolsScope::Active, Some("bare")).is_empty());

        // Bumping the pool's current moves the resolved dir (read side
        // follows `current`, never history).
        save_resource_version(
            "tools",
            "b",
            &[VersionFile {
                rel_path: "run-b.sh".into(),
                bytes: b"y".to_vec(),
            }],
        )
        .unwrap();
        assert_eq!(
            tools_paths(ToolsScope::Active, Some("worker")),
            vec![tmp.path().join("tools/b/v2")]
        );
        assert_eq!(read_resource_meta("tools", "b").unwrap().current, 2);
    }
}
