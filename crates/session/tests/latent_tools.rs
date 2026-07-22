//! Integration tests: latent tools (ssh_pty, chrome_headless) are hidden from
//! the model by default and only appear when their owning skill is activated.

use opencoder_core::{CapabilitiesConfig, ToolFilter};
use opencoder_session::tools::{latent, registry, schema_for};

/// Build the set of tool names that would be sent to the model, given an
/// agent tool filter, capabilities, and an optional skill body.
fn visible_tool_names(
    agent_filter: &ToolFilter,
    caps: &CapabilitiesConfig,
    skill_body: Option<&str>,
) -> Vec<String> {
    let reg = registry();
    let unlocked = latent::unlocked_from_body(skill_body);
    let allowed: Vec<String> = reg
        .keys()
        .filter(|name| {
            agent_filter.allows(name)
                && caps.tool_enabled(name)
                && (!latent::is_latent_tool(name) || unlocked.contains(name.as_str()))
        })
        .cloned()
        .collect();
    let mut sorted = allowed;
    sorted.sort();
    sorted
}

#[test]
fn latent_tools_hidden_by_default() {
    // Use ToolFilter::All so the agent filter doesn't hide them — the only
    // thing hiding them should be the latent filter.
    let filter = ToolFilter::All;
    let caps = CapabilitiesConfig::default();

    let names = visible_tool_names(&filter, &caps, None);

    // ssh_pty and chrome_headless must NOT appear.
    assert!(
        !names.contains(&"ssh_pty".to_string()),
        "ssh_pty should be hidden by default, got: {names:?}"
    );
    assert!(
        !names.contains(&"chrome_headless".to_string()),
        "chrome_headless should be hidden by default, got: {names:?}"
    );

    // But normal tools like bash/read should appear.
    assert!(names.contains(&"bash".to_string()));
    assert!(names.contains(&"read".to_string()));
}

#[test]
fn latent_tools_unlocked_by_skill_body() {
    let filter = ToolFilter::All;
    let caps = CapabilitiesConfig::default();

    // Simulate the ssh-pty skill body being active.
    let body = "# ssh-pty skill\n\nUse ssh_pty for persistent SSH.";
    let names = visible_tool_names(&filter, &caps, Some(body));

    assert!(
        names.contains(&"ssh_pty".to_string()),
        "ssh_pty should be unlocked when ssh-pty skill is active, got: {names:?}"
    );
    // chrome_headless should still be hidden.
    assert!(
        !names.contains(&"chrome_headless".to_string()),
        "chrome_headless should still be hidden, got: {names:?}"
    );

    // Now activate chrome-headless too.
    let body2 = "# ssh-pty skill\nssh_pty\n\n# chrome-headless skill\nchrome_headless";
    let names2 = visible_tool_names(&filter, &caps, Some(body2));
    assert!(names2.contains(&"ssh_pty".to_string()));
    assert!(names2.contains(&"chrome_headless".to_string()));
}

#[test]
fn latent_tools_appear_in_schema_when_unlocked() {
    let filter = ToolFilter::All;
    let caps = CapabilitiesConfig::default();
    let reg = registry();

    // Without skill: schemas should not include ssh_pty.
    let unlocked = latent::unlocked_from_body(None);
    let allowed: std::collections::HashMap<_, _> = reg
        .iter()
        .filter(|(name, _)| {
            filter.allows(name)
                && caps.tool_enabled(name)
                && (!latent::is_latent_tool(name) || unlocked.contains(name.as_str()))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let schemas = schema_for(&allowed, opencoder_core::AgentKind::Act, &caps);
    let schema_names: Vec<&str> = schemas
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap())
        .collect();
    assert!(!schema_names.contains(&"ssh_pty"));

    // With skill: ssh_pty should appear in schemas.
    let unlocked2 = latent::unlocked_from_body(Some("ssh-pty ssh_pty"));
    let allowed2: std::collections::HashMap<_, _> = reg
        .iter()
        .filter(|(name, _)| {
            filter.allows(name)
                && caps.tool_enabled(name)
                && (!latent::is_latent_tool(name) || unlocked2.contains(name.as_str()))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let schemas2 = schema_for(&allowed2, opencoder_core::AgentKind::Act, &caps);
    let schema_names2: Vec<&str> = schemas2
        .iter()
        .map(|s| s["function"]["name"].as_str().unwrap())
        .collect();
    assert!(schema_names2.contains(&"ssh_pty"));
}
