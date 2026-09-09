use opencoder_core::{agent, Config};
use serde_json::{json, Value};

/// Public version references for both pending and running executions.
pub fn snapshot(config: &Config, runner: &str, name: &str) -> Value {
    agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
        let card = agent::read_agent_meta(name);
        let profile = card.as_ref().and_then(|m| m.harness_profile.as_ref());
        let mut resources = serde_json::Map::new();
        if let Some(card) = &card {
            for (category, name) in [
                ("prompts", &card.current.prompt),
                ("skills", &card.current.skills),
                ("tools", &card.current.tools),
                ("memory", &card.current.memory),
            ] {
                if let Some(name) = name {
                    let version = agent::resource_current_version_dir(category, name)
                        .and_then(|p| p.file_name().map(|v| v.to_string_lossy().into_owned()));
                    resources.insert(category.into(), json!({"name":name,"version":version}));
                }
            }
        }
        json!({"runner":runner,"revision":config.agent.runtime.runners.get(runner).map(|r|r.revision),
            "agent":name,"profile":profile,"profile_revision":profile.and_then(|p|config.agent.runtime.profiles.get(p)).map(|p|p.revision),
            "resources":resources})
    })
}
