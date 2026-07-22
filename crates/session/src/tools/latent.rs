//! Latent tool gating: tools that exist in the registry but are hidden from
//! the model until a corresponding skill is activated via `{$skill-name}`.
//!
//! This is the third filtering layer (after `ToolFilter` and
//! `CapabilitiesConfig`). A latent tool passes the agent allowlist and
//! capability gate but is still withheld unless its owning skill's name is in
//! the session's `active_skill_names` set.

use std::collections::HashSet;

/// All tool names that are latent (hidden until their skill is activated).
const LATENT_TOOLS: &[&str] = &["ssh_pty", "chrome_headless"];

/// True when `name` is a latent tool.
pub fn is_latent_tool(name: &str) -> bool {
    LATENT_TOOLS.contains(&name)
}

/// Returns the tool names unlocked by activating `skill_name`.
/// Returns an empty slice for unknown / non-latent skills.
pub fn latent_tools_for_skill(skill_name: &str) -> &'static [&'static str] {
    match skill_name {
        "ssh-pty" => &["ssh_pty"],
        "chrome-headless" => &["chrome_headless"],
        _ => &[],
    }
}

/// Compute the set of latent tool names that should be unlocked, given the
/// currently active skill names. Non-latent tools are never included.
pub fn unlocked_tools(skill_names: &HashSet<String>) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    for name in skill_names {
        for tool in latent_tools_for_skill(name) {
            out.insert(*tool);
        }
    }
    out
}

/// Derive unlocked latent tools from a skill prompt body. Used by the runner
/// to unlock tools without a separate `active_skill_names` registry — the body
/// text already contains skill-specific identifiers that we match against.
pub fn unlocked_from_body(body: Option<&str>) -> HashSet<&'static str> {
    let mut out = HashSet::new();
    if let Some(b) = body {
        let prefix: String = b.chars().take(500).collect();
        if prefix.contains("ssh_pty") || prefix.contains("ssh-pty") {
            for t in latent_tools_for_skill("ssh-pty") {
                out.insert(*t);
            }
        }
        if prefix.contains("chrome_headless") || prefix.contains("chrome-headless") {
            for t in latent_tools_for_skill("chrome-headless") {
                out.insert(*t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_pty_and_chrome_are_latent() {
        assert!(is_latent_tool("ssh_pty"));
        assert!(is_latent_tool("chrome_headless"));
    }

    #[test]
    fn normal_tools_not_latent() {
        assert!(!is_latent_tool("bash"));
        assert!(!is_latent_tool("read"));
        assert!(!is_latent_tool("web_fetch"));
    }

    #[test]
    fn skill_to_tool_mapping() {
        assert_eq!(latent_tools_for_skill("ssh-pty"), &["ssh_pty"]);
        assert_eq!(
            latent_tools_for_skill("chrome-headless"),
            &["chrome_headless"]
        );
        assert!(latent_tools_for_skill("unknown").is_empty());
    }

    #[test]
    fn unlocked_tools_from_skill_names() {
        let mut names = HashSet::new();
        names.insert("ssh-pty".to_string());
        let unlocked = unlocked_tools(&names);
        assert!(unlocked.contains("ssh_pty"));
        assert!(!unlocked.contains("chrome_headless"));

        names.insert("chrome-headless".to_string());
        let unlocked = unlocked_tools(&names);
        assert!(unlocked.contains("ssh_pty"));
        assert!(unlocked.contains("chrome_headless"));
    }

    #[test]
    fn unlocked_from_body_detects_skills() {
        let body = Some("# ssh-pty skill\n\nYou have ssh_pty...");
        let unlocked = unlocked_from_body(body);
        assert!(unlocked.contains("ssh_pty"));

        let body2 = Some("chrome-headless skill chrome_headless tool");
        let unlocked2 = unlocked_from_body(body2);
        assert!(unlocked2.contains("chrome_headless"));
    }

    #[test]
    fn unlocked_from_body_none_when_no_skill() {
        assert!(unlocked_from_body(None).is_empty());
        assert!(unlocked_from_body(Some("random text")).is_empty());
    }

    #[test]
    fn unknown_skill_unlocks_nothing() {
        let mut names = HashSet::new();
        names.insert("bogus".to_string());
        assert!(unlocked_tools(&names).is_empty());
    }
}
