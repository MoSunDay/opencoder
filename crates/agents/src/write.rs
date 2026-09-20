//! Version + reference-card writes: the mutation core of the agents tree.

use std::io;
use std::path::PathBuf;

use opencoder_core::agent::{
    agent_dir, agents_dir, read_agent_meta, read_resource_meta, validate_agent_name,
    validate_resource_name, AgentHistoryEntry, AgentMeta, AgentRefs, ResourceMeta, RunMode,
    AGENT_CATEGORIES,
};

use crate::io::{
    atomic_write, atomic_write_json, invalid_input, not_found, now_rfc3339, sync_dir_best_effort,
};
use crate::references::references_snapshot;

/// One file inside a version dir: `rel_path` is relative to the version
/// dir (nested dirs allowed), `bytes` the raw content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionFile {
    pub rel_path: String,
    pub bytes: Vec<u8>,
}

/// A `rel_path` is legal when non-empty, relative (no leading `/`), and
/// confined to the version dir (no `..` component) — path traversal is
/// rejected before any filesystem work happens.
fn validate_rel_path(rel: &str) -> io::Result<()> {
    crate::resources::model::validate_path(rel)
}

/// Resource dir `<agents_root>/<cat>/<name>` (category + name validated
/// first — no traversal paths). Shared with the rollback pointer switch.
pub(crate) fn resource_dir(cat: &str, name: &str) -> io::Result<PathBuf> {
    validate_resource_name(cat, name).map_err(invalid_input)?;
    let root = agents_dir().ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    let path = root.join(cat).join(name);
    crate::resources::filesystem::check_path(&path)?;
    Ok(path)
}

/// Default meta for a first-time resource: `current: 0` (absent), empty
/// history, both timestamps now.
fn default_resource_meta(name: &str) -> ResourceMeta {
    let now = now_rfc3339();
    ResourceMeta {
        owner_agent: None,
        name: name.to_string(),
        created_at: now.clone(),
        updated_at: now,
        current: 0,
        history: Vec::new(),
    }
}

/// Next version number: `max(history ∪ {current}) + 1` — numbers are
/// never reused, even after a rollback moved `current` backwards.
fn next_version(meta: &ResourceMeta) -> u32 {
    meta.history
        .iter()
        .copied()
        .chain(std::iter::once(meta.current))
        .max()
        .unwrap_or(0)
        + 1
}

/// Save a new version of a pool resource: all `files` are written under a
/// `.tmp-v{n}.<pid>` temp dir sibling, then renamed into place as
/// `<cat>/<name>/v{n}` (atomic dir swap; `AlreadyExists` if the target
/// exists). Finally `meta.json` is updated atomically — `current: n`,
/// `history += [n]`, `updated_at`. On any failure the temp dir is removed
/// and the meta is untouched. Returns the new version number.
pub fn save_resource_version(cat: &str, name: &str, files: &[VersionFile]) -> io::Result<u32> {
    save_shared_version(cat, name, files, None)
}

/// A legacy HTTP create/update checks existence inside the same write lock.
pub fn save_shared_version(
    cat: &str,
    name: &str,
    files: &[VersionFile],
    exists: Option<bool>,
) -> io::Result<u32> {
    let _lock = crate::resources::lock::write_lock()?;
    crate::resources::require_shared(cat, name)?;
    let present = resource_dir(cat, name)?.join("meta.json").exists();
    if exists == Some(false) && present {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "resource already exists",
        ));
    }
    if exists == Some(true) && !present {
        return Err(not_found("resource does not exist"));
    }

    if !AGENT_CATEGORIES.contains(&cat) {
        return Err(invalid_input(format!("未知资源类别: {cat}")));
    }
    validate_resource_name(cat, name).map_err(invalid_input)?;
    for file in files {
        validate_rel_path(&file.rel_path)?;
    }
    let dir = resource_dir(cat, name)?;
    crate::resources::filesystem::check_path(&dir.join("meta.json"))?;
    std::fs::create_dir_all(&dir)?;
    let mut meta = read_resource_meta(cat, name).unwrap_or_else(|| default_resource_meta(name));
    let next = next_version(&meta);
    let dest = dir.join(format!("v{next}"));
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("version dir exists: {}", dest.display()),
        ));
    }
    let temp = dir.join(format!(".tmp-v{next}.{}", std::process::id()));
    let build = || -> io::Result<()> {
        std::fs::create_dir(&temp)?;
        for file in files {
            let target = temp.join(&file.rel_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            atomic_write(&target, &file.bytes)?;
        }
        std::fs::rename(&temp, &dest)
    };
    if let Err(e) = build() {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(e);
    }
    sync_dir_best_effort(&dir);
    meta.current = next;
    meta.history.push(next);
    meta.updated_at = now_rfc3339();
    if let Err(error) = atomic_write_json(&dir.join("meta.json"), &meta) {
        let _ = std::fs::remove_dir_all(&dest);
        return Err(error);
    }
    Ok(next)
}

/// The four reference fields as `(field, value)` pairs, in fixed order.
fn ref_fields(refs: &AgentRefs) -> [(&'static str, Option<String>); 4] {
    [
        ("prompt", refs.prompt.clone()),
        ("skills", refs.skills.clone()),
        ("tools", refs.tools.clone()),
        ("memory", refs.memory.clone()),
    ]
}

/// Create `<name>/meta.json` — a thin reference card with empty history
/// and a freshly scanned `references` snapshot. `AlreadyExists` if a card
/// is already there; the agents root / agent dir are created as needed.
pub fn create_agent(name: &str, refs: AgentRefs) -> io::Result<()> {
    create_agent_with_harness(name, refs, Default::default())
}

pub fn create_agent_with_harness(
    name: &str,
    refs: AgentRefs,
    harness: opencoder_core::harness::Harness,
) -> io::Result<()> {
    create_agent_with_profile(name, refs, harness, None, RunMode::default())
}

pub fn create_agent_with_profile(
    name: &str,
    refs: AgentRefs,
    harness: opencoder_core::harness::Harness,
    profile: Option<String>,
    run_mode: RunMode,
) -> io::Result<()> {
    let _lock = crate::resources::lock::write_lock()?;
    crate::resources::validate_refs(name, &refs)?;
    if let Some(profile) = &profile {
        validate_agent_name(profile).map_err(invalid_input)?;
    }
    if profile.is_some() && harness != opencoder_core::harness::Harness::Codex {
        return Err(invalid_input("Codex profile requires Codex harness"));
    }
    validate_agent_name(name).map_err(invalid_input)?;
    let dir = agent_dir(name).ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    let card = dir.join("meta.json");
    crate::resources::filesystem::check_path(&card)?;
    if card.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent `{name}` already exists"),
        ));
    }
    let now = now_rfc3339();
    let meta = AgentMeta {
        harness_profile: profile,
        harness,
        run_mode,
        name: name.to_string(),
        created_at: now.clone(),
        updated_at: now,
        current: refs,
        history: Vec::new(),
        references: Default::default(),
    };
    let references = references_snapshot(&meta);
    let meta = AgentMeta { references, ..meta };
    std::fs::create_dir_all(&dir)?;
    atomic_write_json(&card, &meta)
}

/// Rewrite a card's references: one `AgentHistoryEntry{at, field, from,
/// to}` is appended per **changed** field (unchanged fields contribute
/// nothing), `updated_at` bumps, and the `references` snapshot refreshes.
/// The card must exist (`NotFound` otherwise).
pub fn update_agent_refs(name: &str, refs: AgentRefs) -> io::Result<()> {
    update_agent_settings(name, Some(refs), None)
}

pub fn update_agent_settings(
    name: &str,
    refs: Option<AgentRefs>,
    harness: Option<opencoder_core::harness::Harness>,
) -> io::Result<()> {
    update_agent_with_profile(name, refs, harness, None, None)
}

pub fn update_agent_with_profile(
    name: &str,
    refs: Option<AgentRefs>,
    harness: Option<opencoder_core::harness::Harness>,
    profile: Option<Option<String>>,
    run_mode: Option<RunMode>,
) -> io::Result<()> {
    let _lock = crate::resources::lock::write_lock()?;
    validate_agent_name(name).map_err(invalid_input)?;
    let dir = agent_dir(name).ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    crate::resources::filesystem::check_path(&dir.join("meta.json"))?;
    let builtin = opencoder_core::builtin_agents()
        .iter()
        .any(|a| a.name == name);
    let mut meta = match read_agent_meta(name) {
        Some(meta) => meta,
        None if builtin => AgentMeta {
            name: name.into(),
            ..Default::default()
        },
        None => return Err(not_found(format!("unknown agent: {name}"))),
    };
    let refs = refs.unwrap_or_else(|| meta.current.clone());
    crate::resources::validate_refs(name, &refs)?;
    let now = now_rfc3339();
    let profile = if harness == Some(opencoder_core::harness::Harness::Opencoder) {
        Some(None)
    } else {
        profile
    };
    if let Some(profile) = profile {
        if let Some(name) = &profile {
            validate_agent_name(name).map_err(invalid_input)?;
            if harness.unwrap_or(meta.harness) != opencoder_core::harness::Harness::Codex {
                return Err(invalid_input("Codex profile requires Codex harness"));
            }
        }
        if meta.harness_profile != profile {
            meta.history.push(AgentHistoryEntry {
                at: now.clone(),
                field: "harness_profile".into(),
                from: meta.harness_profile.clone(),
                to: profile.clone(),
            });
            meta.harness_profile = profile;
        }
    }
    if let Some(harness) = harness {
        if meta.harness != harness {
            meta.history.push(AgentHistoryEntry {
                at: now.clone(),
                field: "harness".into(),
                from: Some(meta.harness.as_str().into()),
                to: Some(harness.as_str().into()),
            });
            meta.harness = harness;
        }
    }
    // Run mode mirrors harness: `Some(new)` appends one history entry when
    // the value actually changes; `None` leaves the card untouched.
    if let Some(run_mode) = run_mode {
        if meta.run_mode != run_mode {
            meta.history.push(AgentHistoryEntry {
                at: now.clone(),
                field: "run_mode".into(),
                from: Some(meta.run_mode.as_str().into()),
                to: Some(run_mode.as_str().into()),
            });
            meta.run_mode = run_mode;
        }
    }
    std::fs::create_dir_all(&dir)?;
    let changed = ref_fields(&meta.current)
        .into_iter()
        .zip(ref_fields(&refs))
        .filter_map(|((field, from), (_, to))| {
            (from != to).then(|| AgentHistoryEntry {
                at: now.clone(),
                field: field.to_string(),
                from,
                to,
            })
        })
        .collect::<Vec<_>>();
    meta.history.extend(changed);
    meta.current = refs;
    meta.updated_at = now;
    meta.references = references_snapshot(&meta);
    atomic_write_json(&dir.join("meta.json"), &meta)
}

/// Remove an agent card (`<name>/` directory). Missing dir ⇒ `Ok` —
/// idempotent. The caller (web layer) clears the active marker first;
/// resource pools are shared and never touched here.
pub fn delete_agent(name: &str) -> io::Result<()> {
    let _lock = crate::resources::lock::write_lock()?;
    validate_agent_name(name).map_err(invalid_input)?;
    let Some(dir) = agent_dir(name) else {
        return Ok(());
    };
    crate::resources::filesystem::check_path(&dir)?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests;
