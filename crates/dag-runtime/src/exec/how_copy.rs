//! Frozen Agent prompts and execution-local how.md copies; no shared-pool writes.
use super::StepCtx;
use anyhow::{Context, Result};
use opencoder_core::agent::{self, Agent};
use opencoder_dag::{StepKind, StepSpec};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct FrozenAgent {
    agent: Agent,
    how: String,
}

fn optional_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Called before scheduling, so every instance shares the same frozen original.
pub(crate) fn freeze(root: &Path, run: &str, step: &StepSpec) -> Result<()> {
    let StepKind::Agent { agent: name, .. } = step.kind.executable() else {
        return Ok(());
    };
    let dir =
        opencoder_dag::artifacts::step_dir(root, run, &step.name).map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("frozen-agent.json");
    if path.exists() {
        serde_json::from_slice::<FrozenAgent>(&std::fs::read(path)?)?;
        return Ok(());
    }
    let name = name.as_deref().unwrap_or("act");
    let agent =
        opencoder_core::resolve_agent(name).with_context(|| format!("unknown agent {name}"))?;
    let how = if opencoder_core::builtin_agents()
        .iter()
        .any(|a| a.name == name)
    {
        String::new()
    } else {
        let pool = agent::read_agent_meta(name)
            .and_then(|m| m.current.prompt)
            .context("agent prompt resource missing")?;
        let source = agent::resource_current_version_dir("prompts", &pool)
            .context("agent prompt version missing")?;
        optional_text(&source.join("how.md"))?
    };
    crate::checkpoint::write(&path, &serde_json::to_vec(&FrozenAgent { agent, how })?)
}

pub(crate) fn prepare(ctx: &StepCtx) -> Result<Agent> {
    let dir = ctx.dir().map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&dir)?;
    let source =
        opencoder_dag::artifacts::step_dir(&ctx.workflow_root, &ctx.run_id, &ctx.step.name)
            .map_err(anyhow::Error::msg)?
            .join("frozen-agent.json");
    // Direct executor callers also freeze resources before creating a copy.
    if !source.exists() {
        freeze(&ctx.workflow_root, &ctx.run_id, &ctx.step)?;
    }
    let bytes = std::fs::read(source)?;
    let frozen: FrozenAgent = serde_json::from_slice(&bytes)?;
    let how_path = dir.join("how.md");
    if !how_path.exists() {
        let common = match &ctx.step.kind {
            StepKind::Agent { how_append, .. } => how_append.as_deref(),
            _ => None,
        };
        let how = append(
            &frozen.how,
            common,
            ctx.instance_input.as_ref().and_then(|v| v.as_str()),
        );
        crate::checkpoint::write(&how_path, how.as_bytes())?;
    }
    crate::checkpoint::write(&dir.join("agent.json"), &bytes)?;
    load(&dir)
}

/// Host and container use exactly the same prompt composition and local copy.
pub fn load(dir: &Path) -> Result<Agent> {
    let frozen: FrozenAgent = serde_json::from_slice(&std::fs::read(dir.join("agent.json"))?)?;
    let how = std::fs::read_to_string(dir.join("how.md"))?;
    let mut agent = frozen.agent;
    agent.prompt = replace_how(&agent.prompt, &frozen.how, &how);
    Ok(agent)
}

fn append(original: &str, common: Option<&str>, text: Option<&str>) -> String {
    let mut result = original.to_string();
    for extra in [common, text].into_iter().flatten() {
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(extra);
    }
    result
}

fn replace_how(prompt: &str, original: &str, how: &str) -> String {
    if how == original {
        return prompt.to_string();
    }
    let section = format!("# How\n{}", how.trim());
    if !original.trim().is_empty() {
        prompt.replacen(&format!("# How\n{}", original.trim()), &section, 1)
    } else if how.trim().is_empty() {
        prompt.to_string()
    } else {
        format!("{prompt}\n\n{section}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compose_preserves_sections_and_appends_once() {
        let how = append("original\n", Some("common"), Some("instance"));
        assert_eq!(how, "original\n\n\ncommon\n\ninstance");
        assert_eq!(
            replace_how(
                "# Soul\nsoul\n\n# How\noriginal\n\n# Output\njson",
                "original\n",
                &how
            ),
            "# Soul\nsoul\n\n# How\noriginal\n\n\ncommon\n\ninstance\n\n# Output\njson"
        );
        assert_eq!(replace_how("builtin", "", "note"), "builtin\n\n# How\nnote");
    }
}
