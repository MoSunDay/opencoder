//! Materialize the selected agent resources inside the execution workspace.
use crate::SessionState;
use anyhow::{ensure, Context, Result};
use opencoder_core::{agent, Agent, AgentKind, AgentMode, ToolFilter};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RESOURCE_INDEX: &str = "\n\n## Agent resource files\nRead referenced files from these exact paths. Resolve their relative links from the containing file. Execute tool scripts using the listed absolute paths if shell startup changes PATH.\n";

/// Stable instructions exclude the per-session materialized path index.
pub fn instruction_text(prompt: &str) -> &str {
    prompt
        .rsplit_once(RESOURCE_INDEX)
        .map_or(prompt, |(text, _)| text)
}

pub fn invalidate_native(session: &mut SessionState) {
    if session.harness.harness == opencoder_core::harness::Harness::Opencoder {
        session.agent.prompt = instruction_text(&session.agent.prompt).to_owned();
        session.harness.resource_root = None;
    }
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    name: String,
    kind: AgentKind,
    mode: AgentMode,
    description: String,
    prompt: String,
    tools: ToolFilter,
    tools_path: Vec<PathBuf>,
    skill_roots: Vec<PathBuf>,
}

pub fn restore_agent(root: &Path) -> Result<Agent> {
    let s: Snapshot = serde_json::from_slice(&std::fs::read(root.join("snapshot.json"))?)?;
    Ok(Agent {
        name: s.name,
        kind: s.kind,
        mode: s.mode,
        description: s.description,
        prompt: s.prompt,
        tools: s.tools,
    })
}

pub fn prepare(session: &mut SessionState) -> Result<()> {
    if let Some(root) = &session.harness.resource_root {
        let s: Snapshot = serde_json::from_slice(
            &std::fs::read(root.join("snapshot.json"))
                .context("agent resource snapshot missing")?,
        )?;
        if s.name != session.agent.name {
            session.harness.resource_root = None;
            return prepare(session);
        }
        session.agent.prompt = s.prompt;
        session.tools_path = s.tools_path;
        session.skill_roots = s.skill_roots;
        return Ok(());
    }
    let card: Option<agent::AgentMeta> = match agent::agent_dir(&session.agent.name) {
        Some(dir) => match std::fs::read(dir.join("meta.json")) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes).context("invalid agent resource card or harness")?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        },
        None => None,
    };
    if card.is_none() && session.harness.harness == opencoder_core::harness::Harness::Opencoder {
        return Ok(());
    }
    let workdir = std::fs::canonicalize(&session.working_dir)?;
    let root = workdir
        .join(".opencoder/runtime")
        .join(&session.id)
        .join(&session.agent.name)
        .join(ulid::Ulid::new().to_string());
    for ancestor in root.ancestors().take_while(|p| *p != workdir) {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "runtime resource path cannot be a symlink"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    exclude_runtime_from_git(&session.working_dir)?;
    std::fs::create_dir_all(root.parent().context("snapshot parent missing")?)?;
    let staging = root.with_extension(format!("staging-{}", ulid::Ulid::new()));
    std::fs::create_dir(&staging)?;
    let result = (|| -> Result<Snapshot> {
        let mut index = String::from(RESOURCE_INDEX);
        let mut tools_path = Vec::new();
        let mut skill_roots = Vec::new();
        if let Some(card) = card {
            for (category, name) in [
                ("prompts", card.current.prompt),
                ("skills", card.current.skills),
                ("tools", card.current.tools),
                ("memory", card.current.memory),
            ] {
                if let Some(name) = name {
                    let source = agent::resource_current_version_dir(category, &name)
                        .with_context(|| format!("agent resource missing: {category}/{name}"))?;
                    let relative = PathBuf::from(category)
                        .join(name)
                        .join(source.file_name().context("resource version missing")?);
                    copy_tree(&source, &staging.join(&relative))?;
                    let path = root.join(&relative);
                    index.push_str(&format!("- {category}: {}\n", path.display()));
                    if category == "tools" {
                        tools_path.push(path.clone());
                    }
                    if category == "skills" {
                        skill_roots.push(path);
                    }
                }
            }
        }
        for source in &session.tools_path {
            let resource = source
                .parent()
                .and_then(Path::file_name)
                .context("tool resource name missing")?;
            let version = source.file_name().context("tool version missing")?;
            let relative = PathBuf::from("tools").join(resource).join(version);
            let path = root.join(&relative);
            if !tools_path.contains(&path) {
                copy_tree(source, &staging.join(&relative))?;
                index.push_str(&format!("- tools: {}\n", path.display()));
                tools_path.push(path);
            }
        }
        let s = Snapshot {
            name: session.agent.name.clone(),
            kind: session.agent.kind,
            mode: session.agent.mode,
            description: session.agent.description.clone(),
            prompt: format!("{}{}", session.agent.prompt, index),
            tools: session.agent.tools.clone(),
            tools_path,
            skill_roots,
        };
        std::fs::write(
            staging.join("snapshot.json"),
            serde_json::to_vec_pretty(&s)?,
        )?;
        std::fs::rename(&staging, &root)?;
        Ok(s)
    })();
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let s = result?;
    session.agent.prompt = s.prompt;
    session.tools_path = s.tools_path;
    session.skill_roots = s.skill_roots;
    session.harness.resource_root = Some(root);
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(source)?;
    ensure!(
        !meta.file_type().is_symlink(),
        "agent resource cannot be a symlink: {}",
        source.display()
    );
    if meta.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        ensure!(meta.is_file(), "agent resource must be a regular file");
        std::fs::copy(source, target)?;
        std::fs::set_permissions(target, meta.permissions())?;
    }
    Ok(())
}

fn exclude_runtime_from_git(workdir: &Path) -> Result<()> {
    let result = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(workdir)
        .stderr(std::process::Stdio::null())
        .output();
    let result = match result {
        Ok(result) => result,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !result.status.success() {
        return Ok(());
    }
    let path = workdir.join(String::from_utf8(result.stdout)?.trim());
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let rule = "**/.opencoder/runtime/";
    if !text.lines().any(|line| line == rule) {
        text.push_str(&format!("\n{rule}\n"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        opencoder_core::atomic_write(&path, text.as_bytes())?;
    }
    Ok(())
}
