//! Version + reference-card writes: the mutation core of the agents tree.

use std::io;
use std::path::{Path, PathBuf};

use opencoder_core::agent::{
    agent_dir, agents_dir, read_agent_meta, read_resource_meta, validate_agent_name,
    validate_resource_name, AgentHistoryEntry, AgentMeta, AgentRefs, ResourceMeta,
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
    if rel.is_empty() {
        return Err(invalid_input("rel_path 不能为空"));
    }
    if rel.starts_with('/') {
        return Err(invalid_input(format!("rel_path 不能是绝对路径: {rel}")));
    }
    let confined = Path::new(rel).components().all(|c| {
        !matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    });
    if !confined {
        return Err(invalid_input(format!("rel_path 不能包含 ..: {rel}")));
    }
    Ok(())
}

/// Resource dir `<agents_root>/<cat>/<name>` (category + name validated
/// first — no traversal paths). Shared with the rollback pointer switch.
pub(crate) fn resource_dir(cat: &str, name: &str) -> io::Result<PathBuf> {
    validate_resource_name(cat, name).map_err(invalid_input)?;
    let root = agents_dir().ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    Ok(root.join(cat).join(name))
}

/// Default meta for a first-time resource: `current: 0` (absent), empty
/// history, both timestamps now.
fn default_resource_meta(name: &str) -> ResourceMeta {
    let now = now_rfc3339();
    ResourceMeta {
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
    if !AGENT_CATEGORIES.contains(&cat) {
        return Err(invalid_input(format!("未知资源类别: {cat}")));
    }
    validate_resource_name(cat, name).map_err(invalid_input)?;
    for file in files {
        validate_rel_path(&file.rel_path)?;
    }
    let dir = resource_dir(cat, name)?;
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
        std::fs::create_dir_all(&temp)?;
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
    atomic_write_json(&dir.join("meta.json"), &meta)?;
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
    validate_agent_name(name).map_err(invalid_input)?;
    let dir = agent_dir(name).ok_or_else(|| not_found("cannot resolve ~/.opencode"))?;
    let card = dir.join("meta.json");
    if card.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("agent `{name}` already exists"),
        ));
    }
    let now = now_rfc3339();
    let meta = AgentMeta {
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
    validate_agent_name(name).map_err(invalid_input)?;
    let dir = agent_dir(name).ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    let Some(mut meta) = read_agent_meta(name) else {
        return Err(not_found(format!("unknown agent: {name}")));
    };
    let now = now_rfc3339();
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
    validate_agent_name(name).map_err(invalid_input)?;
    let Some(dir) = agent_dir(name) else {
        return Ok(());
    };
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scoped;

    fn vf(rel: &str) -> VersionFile {
        VersionFile {
            rel_path: rel.into(),
            bytes: rel.as_bytes().to_vec(),
        }
    }

    fn meta_of(cat: &str, name: &str) -> ResourceMeta {
        read_resource_meta(cat, name).unwrap()
    }

    #[test]
    fn versions_increment_and_never_reuse() {
        let (tmp, _g) = scoped();
        assert_eq!(
            save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
            1
        );
        assert_eq!(
            save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
            2
        );
        assert_eq!(
            save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
            3
        );
        crate::rollback::rollback_resource("prompts", "pack", 1).unwrap();
        assert_eq!(
            save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap(),
            4
        );
        let meta = meta_of("prompts", "pack");
        assert_eq!(meta.current, 4);
        assert_eq!(meta.history, vec![1, 2, 3, 4]);
        for v in 1..=4 {
            assert!(tmp.path().join(format!("prompts/pack/v{v}")).is_dir());
        }
        // Unknown category rejected before touching the fs.
        assert_eq!(
            save_resource_version("nope", "pack", &[vf("x")])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn failed_save_leaves_no_temp_and_meta_unchanged() {
        let (tmp, _g) = scoped();
        save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
        let before = meta_of("tools", "kit");
        let err = save_resource_version("tools", "kit", &[vf("../escape")]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let kit = tmp.path().join("tools/kit");
        let temps: Vec<_> = std::fs::read_dir(&kit)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(temps.is_empty(), "temp dirs left behind: {temps:?}");
        assert_eq!(meta_of("tools", "kit"), before);
        // Empty and absolute rel_paths are rejected too.
        assert!(save_resource_version("tools", "kit", &[vf("")]).is_err());
        assert!(save_resource_version("tools", "kit", &[vf("/etc/x")]).is_err());
    }

    #[test]
    fn create_update_card_history() {
        let (_tmp, _g) = scoped();
        save_resource_version("prompts", "pack", &[vf("soul.md")]).unwrap();
        save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
        let first = AgentRefs {
            prompt: Some("pack".into()),
            skills: None,
            tools: None,
            memory: None,
        };
        create_agent("work", first.clone()).unwrap();
        let card = read_agent_meta("work").unwrap();
        assert!(card.history.is_empty());
        assert_eq!(card.current, first);
        assert_eq!(card.references.prompt_files, vec!["soul"]);
        // Duplicate create rejected.
        assert_eq!(
            create_agent("work", Default::default()).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        // Change two fields → exactly two history entries.
        update_agent_refs(
            "work",
            AgentRefs {
                prompt: Some("pack".into()),
                skills: None,
                tools: Some("kit".into()),
                memory: Some("bank".into()),
            },
        )
        .unwrap();
        let card = read_agent_meta("work").unwrap();
        let fields: Vec<&str> = card.history.iter().map(|h| h.field.as_str()).collect();
        assert_eq!(fields, vec!["tools", "memory"]);
        assert_eq!(card.history[0].from, None);
        assert_eq!(card.history[0].to.as_deref(), Some("kit"));
        assert_eq!(card.references.tools, vec!["run.sh"]);
        // Identical refs → no new history entries.
        update_agent_refs("work", card.current.clone()).unwrap();
        assert_eq!(read_agent_meta("work").unwrap().history.len(), 2);
        // Unknown card rejected.
        assert_eq!(
            update_agent_refs("ghost", Default::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        // Reserved / invalid names rejected.
        assert!(create_agent("active", Default::default()).is_err());
        assert!(create_agent("../x", Default::default()).is_err());
    }

    #[test]
    fn delete_agent_is_idempotent() {
        let (tmp, _g) = scoped();
        create_agent("gone", Default::default()).unwrap();
        assert!(tmp.path().join("gone").is_dir());
        delete_agent("gone").unwrap();
        delete_agent("gone").unwrap();
        assert!(!tmp.path().join("gone").exists());
        assert!(delete_agent("never-there").is_ok());
        assert!(delete_agent("active").is_err());
    }
}
