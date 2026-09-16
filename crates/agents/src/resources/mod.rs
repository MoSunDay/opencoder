//! Agent-identity resource editing, with copy-on-write ownership and optimistic concurrency.
pub mod filesystem;
pub mod lock;
pub mod model;
mod transaction;

use crate::io::{invalid_input, not_found};
use filesystem::{read_json, root};
pub use model::{Baseline, FileChange, RestoreRequest, SaveRequest};
use opencoder_core::agent::{
    validate_agent_name, validate_resource_name, AgentMeta, AgentRefs, ResourceMeta,
};
use serde::Serialize;
use std::io;
pub use transaction::{restore, save};

#[derive(Serialize)]
pub struct ResourceView {
    pub ok: bool,
    pub agent: String,
    pub category: String,
    pub baseline: Baseline,
    pub versions: Vec<u32>,
    pub files: Vec<FileChange>,
    pub read_only: bool,
    pub builtin_prompt: Option<String>,
    pub tool_filter: Option<opencoder_core::agent::ToolFilter>,
}

pub(crate) fn reference<'a>(refs: &'a AgentRefs, cat: &str) -> io::Result<&'a Option<String>> {
    match cat {
        "prompts" => Ok(&refs.prompt),
        "skills" => Ok(&refs.skills),
        "tools" => Ok(&refs.tools),
        "memory" => Ok(&refs.memory),
        _ => Err(invalid_input("unknown resource category")),
    }
}
pub(crate) fn set_reference(refs: &mut AgentRefs, cat: &str, name: String) {
    *match cat {
        "prompts" => &mut refs.prompt,
        "skills" => &mut refs.skills,
        "tools" => &mut refs.tools,
        _ => &mut refs.memory,
    } = Some(name);
}
pub(crate) fn card(name: &str) -> io::Result<AgentMeta> {
    validate_agent_name(name).map_err(invalid_input)?;
    let path = root()?.join(name).join("meta.json");
    match read_json::<AgentMeta>(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound && builtin(name).is_some() => Ok(AgentMeta {
            name: name.into(),
            ..Default::default()
        }),
        Ok(mut meta) => {
            meta.name = name.into();
            Ok(meta)
        }
        Err(error) => Err(error),
    }
}
pub(crate) fn builtin(name: &str) -> Option<opencoder_core::agent::Agent> {
    opencoder_core::builtin_agents()
        .into_iter()
        .find(|a| a.name == name)
}
pub(crate) fn resource_meta(cat: &str, name: &str) -> io::Result<ResourceMeta> {
    validate_resource_name(cat, name).map_err(invalid_input)?;
    read_json(&root()?.join(cat).join(name).join("meta.json"))
}
pub(crate) fn current(meta: &AgentMeta, cat: &str) -> io::Result<(Baseline, Option<ResourceMeta>)> {
    let resource = reference(&meta.current, cat)?.clone();
    let mut entry = resource
        .as_deref()
        .map(|name| resource_meta(cat, name))
        .transpose()?;
    if let Some(entry) = &mut entry {
        if entry
            .owner_agent
            .as_deref()
            .is_some_and(|owner| owner != meta.name)
        {
            return Err(invalid_input("resource belongs to another agent"));
        }
        if entry.current == 0 {
            return Err(invalid_input("resource has no valid current version"));
        }
        // Legacy seed/import metadata may omit history; current is still a valid pin.
        entry.history.push(entry.current);
        entry.history.sort_unstable();
        entry.history.dedup();
    }
    Ok((
        Baseline {
            resource,
            version: entry.as_ref().map_or(0, |m| m.current),
            revision: entry
                .as_ref()
                .map_or_else(String::new, |m| m.updated_at.clone()),
        },
        entry,
    ))
}

pub fn read(name: &str, cat: &str) -> io::Result<ResourceView> {
    let _lock = lock::write_lock()?;
    read_unlocked(name, cat)
}
pub(crate) fn read_unlocked(name: &str, cat: &str) -> io::Result<ResourceView> {
    let card = card(name)?;
    let (baseline, entry) = current(&card, cat)?;
    let files = if let Some(resource) = &baseline.resource {
        filesystem::read_files(
            &root()?
                .join(cat)
                .join(resource)
                .join(format!("v{}", baseline.version)),
        )?
    } else {
        vec![]
    };
    if matches!(cat, "prompts" | "memory")
        && files
            .iter()
            .any(|f| std::str::from_utf8(&f.bytes).is_err() || f.bytes.contains(&0))
    {
        return Err(invalid_input("markdown resource contains non-text data"));
    }
    let builtin = builtin(name);
    Ok(ResourceView {
        ok: true,
        agent: name.into(),
        category: cat.into(),
        baseline,
        versions: entry.map_or_else(Vec::new, |m| m.history),
        files: files.iter().map(FileChange::from).collect(),
        read_only: builtin.is_some(),
        builtin_prompt: builtin
            .as_ref()
            .filter(|_| cat == "prompts")
            .map(|a| a.prompt.clone()),
        tool_filter: builtin.filter(|_| cat == "tools").map(|a| a.tools),
    })
}

/// Called under the root write lock by every card mutation, including legacy APIs.
pub(crate) fn validate_refs(name: &str, refs: &AgentRefs) -> io::Result<()> {
    for cat in opencoder_core::agent::AGENT_CATEGORIES {
        if let Some(resource) = reference(refs, cat)? {
            validate_resource_name(cat, resource).map_err(invalid_input)?;
            match resource_meta(cat, resource) {
                Ok(meta)
                    if meta
                        .owner_agent
                        .as_deref()
                        .is_some_and(|owner| owner != name) =>
                {
                    return Err(invalid_input(
                        "cannot bind another agent's private resource",
                    ))
                }
                // Legacy APIs permit not-yet-created shared references.
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => (),
            }
        }
    }
    Ok(())
}

pub(crate) fn check_baseline(
    card: &AgentMeta,
    cat: &str,
    baseline: &Baseline,
) -> io::Result<Option<ResourceMeta>> {
    if builtin(&card.name).is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "builtin agent resources are read-only",
        ));
    }
    let (actual, meta) = current(card, cat)?;
    if &actual != baseline {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "resource changed; reload before saving",
        ));
    }
    Ok(meta)
}

/// Legacy pool APIs may mutate only shared resources.
pub fn require_shared(cat: &str, name: &str) -> io::Result<()> {
    match resource_meta(cat, name) {
        Ok(meta) if meta.owner_agent.is_some() => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "edit private resources through their agent",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        _ => Ok(()),
    }
}

pub fn delete_shared(cat: &str, name: &str) -> io::Result<()> {
    let _lock = lock::write_lock()?;
    let _ = resource_meta(cat, name)?;
    require_shared(cat, name)?;
    for agent in opencoder_core::agent::list_agents() {
        if reference(&card(&agent)?.current, cat)?.as_deref() == Some(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "resource is referenced by agent cards",
            ));
        }
    }
    let dir = root()?.join(cat).join(name);
    filesystem::check_path(&dir)?;
    std::fs::remove_dir_all(dir)
}

pub fn read_file(cat: &str, name: &str, version: u32, path: &str) -> io::Result<Vec<u8>> {
    model::validate_path(path)?;
    let _lock = lock::write_lock()?;
    validate_resource_name(cat, name).map_err(invalid_input)?;
    if version == 0 {
        return Err(not_found("unknown resource version"));
    }
    let path = root()?
        .join(cat)
        .join(name)
        .join(format!("v{version}"))
        .join(path);
    filesystem::check_path(&path)?;
    std::fs::read(path)
}
