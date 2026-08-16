//! Transient skill-context tail reminder.
//!
//! Skill bodies no longer ship in the system prompt — appending them there
//! rewrote the payload's first bytes on every activation, destabilising
//! provider prompt-prefix caching. Instead every LLM call derives, from
//! session state alone, one synthetic user message appended at the END of
//! the request payload: never recorded to the store, never replayed. It
//! carries (a) a catalog of config-enabled skills (lazy-load hint for
//! `~/.opencoder/skills`) and (b) the active skill's source path (from the
//! `> Source:` prefix `body_with_source` writes), so the model reads the
//! SKILL.md on demand instead of paying its tokens up front.

use std::path::Path;

use opencoder_core::{discover_in, skills_dir, AgentMode, Config, Message};

use crate::SessionState;

/// Extract the source-file path from a `> Source: <path>` prefix line
/// (written by `opencoder_core::body_with_source`). `None` when absent or
/// the path is empty.
pub fn source_path_from_body(body: &str) -> Option<&str> {
    let rest = body.strip_prefix("> Source: ")?;
    let path = rest.split('\n').next()?.trim();
    (!path.is_empty()).then_some(path)
}

/// Enabled-skill catalog for an explicit discovery root: the intersection of
/// `config.enabled_skill_names()` with `discover_in(root)`, sorted by name.
pub fn catalog_entries_in(root: &Path, config: &Config) -> Vec<(String, String)> {
    let enabled = config.enabled_skill_names();
    if enabled.is_empty() {
        return Vec::new();
    }
    discover_in(root)
        .into_iter()
        .filter(|s| enabled.iter().any(|n| n == &s.name))
        .map(|s| (s.name, s.description))
        .collect()
}

/// Enabled-skill catalog for the default skills directory.
pub fn catalog_entries(config: &Config) -> Vec<(String, String)> {
    catalog_entries_in(&skills_dir(), config)
}

/// Pure builder for the reminder text. Empty string when both inputs are
/// empty; otherwise the `[skills]` and/or `[active skill]` sections joined
/// by a blank line.
pub fn reminder_text(catalog: &[(String, String)], active_path: Option<&str>) -> String {
    let mut sections: Vec<String> = Vec::new();
    if !catalog.is_empty() {
        let mut lines = vec![format!(
            "[skills]\nEnabled skills live under {}:",
            skills_dir().display()
        )];
        for (name, description) in catalog {
            lines.push(match description.as_str() {
                "" => format!("- {name}"),
                d => format!("- {name}: {d}"),
            });
        }
        lines.push(
            "When a task matches an enabled skill, read its SKILL.md file \
             under that directory first, then follow it."
                .into(),
        );
        sections.push(lines.join("\n"));
    }
    if let Some(path) = active_path {
        sections.push(format!(
            "[active skill]\nAn active skill is in effect: {path}\n\
             Read that file now (if not already loaded) and follow it."
        ));
    }
    sections.join("\n\n")
}

/// Build the per-call tail reminder message, or `None` when there is nothing
/// to remind about. Only Primary agents get skill context: subagents run
/// scoped tasks, and `workflow` — although itself a Primary-mode agent (see
/// `crates/core/src/agent.rs`) — is the todos scheduler and must not receive
/// it, hence the explicit name check.
pub fn tail_reminder(session: &SessionState) -> Option<Message> {
    if session.agent.mode != AgentMode::Primary || session.agent.name == "workflow" {
        return None;
    }
    let skill_prompt = session.skill_prompt_cloned();
    let active_path = skill_prompt.as_deref().and_then(source_path_from_body);
    let text = reminder_text(&catalog_entries(&session.config), active_path);
    if text.is_empty() {
        return None;
    }
    let mut msg = Message::user(crate::runner::new_id(), text);
    msg.synthetic = true;
    Some(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::config::SkillConfig;
    use opencoder_core::{resolve_agent, Role};
    use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
    use std::sync::Arc;

    fn client() -> Arc<dyn ChatStream> {
        Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "ok".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        )
    }

    #[test]
    fn source_path_from_body_variants() {
        let body = "> Source: /skills/review/SKILL.md\n\nbody";
        assert_eq!(source_path_from_body(body), Some("/skills/review/SKILL.md"));
        assert_eq!(source_path_from_body("no prefix"), None);
        assert_eq!(source_path_from_body("> Source: \n\nb"), None);
    }

    #[test]
    fn catalog_entries_in_intersects_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        for (f, front) in [
            ("beta.md", "---\nname: beta\ndescription: B\n---\nb"),
            ("alpha.md", "---\nname: alpha\ndescription: A\n---\na"),
        ] {
            std::fs::write(dir.path().join(f), front).unwrap();
        }
        let mut cfg = Config::default();
        assert!(
            catalog_entries_in(dir.path(), &cfg).is_empty(),
            "nothing enabled"
        );
        let sc = SkillConfig { enabled: true };
        cfg.skills.insert("beta".into(), sc);
        let got = catalog_entries_in(dir.path(), &cfg);
        assert_eq!(
            got,
            vec![("beta".into(), "B".into())],
            "enabled only, by name"
        );
    }

    #[test]
    fn reminder_text_sections() {
        assert_eq!(reminder_text(&[], None), "", "neither section -> empty");
        let catalog = vec![
            ("alpha".into(), "A skill".into()),
            ("plain".into(), String::new()),
        ];
        let both = reminder_text(&catalog, Some("/skills/x/SKILL.md"));
        let head = "[skills]\nEnabled skills live under";
        assert!(
            both.starts_with(head) && both.contains("- alpha: A skill\n- plain\n"),
            "{both}"
        );
        assert!(
            both.contains("read its SKILL.md file"),
            "guidance line: {both}"
        );
        let active = "\n\n[active skill]\nAn active skill is in effect: /skills/x/SKILL.md";
        assert!(
            both.contains(active),
            "sections joined by a blank line: {both}"
        );
        let catalog_only = reminder_text(&catalog, None);
        assert!(catalog_only.contains("[skills]") && !catalog_only.contains("[active skill]"));
        let active_only = reminder_text(&[], Some("/p"));
        assert!(active_only.starts_with("[active skill]\n"), "{active_only}");
    }

    #[test]
    fn tail_reminder_gating_and_content() {
        let mk = |name: &str| {
            SessionState::new(
                "t",
                resolve_agent(name).unwrap(),
                Config::default(),
                client(),
                Path::new("/tmp").into(),
            )
        };
        let s = mk("act");
        assert!(tail_reminder(&s).is_none(), "nothing to remind about");
        s.set_skill(Some("> Source: /skills/rev/SKILL.md\n\nREV".into()));
        let msg = tail_reminder(&s).expect("primary + Source prefix");
        assert_eq!(msg.role, Role::User);
        assert!(msg.synthetic, "transient, never recorded");
        let text = msg.text();
        assert!(text.contains("[active skill]") && text.contains("/skills/rev/SKILL.md"));
        assert!(
            tail_reminder(&mk("workflow")).is_none(),
            "workflow excluded"
        );
        assert!(tail_reminder(&mk("explore")).is_none(), "subagent excluded");
    }
}
