//! Resolved-reference scanning: what a pool's current version actually
//! contains, snapshotted into an agent card's `references` block.

use std::io;
use std::path::Path;

use opencoder_core::agent::{
    agent_dir, read_agent_meta, resource_current_version_dir, resource_version_dir,
    validate_agent_name, AgentMeta, AgentReferences,
};

use crate::io::{atomic_write_json, invalid_input, not_found, now_rfc3339};

/// Prompt file stems in canonical composition order (`soul`, `how`,
/// `output`).
const PROMPT_FILES: [&str; 3] = ["soul", "how", "output"];

/// List what `<cat>/<name>/v{version}` contains, as stable names:
///
/// - `prompts`: stems of the `soul|how|output` `.md` files present
///   (canonical order, gaps skipped);
/// - `skills`: skill names — top-level `*.md` stems plus direct child
///   dirs carrying a `SKILL.md`, mirroring how
///   `opencoder_core::skill::discover_in` names skills in a root;
/// - `tools`: direct-child file/dir names (excluding `meta.json`);
/// - `memory`: `["memory"]` iff `memory.md` is present.
///
/// A missing dir (or unknown cat/name) scans as empty — reads degrade
/// silently, the agents-root philosophy.
pub fn scan_resource(cat: &str, name: &str, version: u32) -> Vec<String> {
    match resource_version_dir(cat, name, version) {
        Some(dir) if dir.is_dir() => scan_dir(cat, &dir),
        _ => Vec::new(),
    }
}

/// Dispatch a category scan over an existing dir.
fn scan_dir(cat: &str, dir: &Path) -> Vec<String> {
    match cat {
        "prompts" => scan_prompts(dir),
        "skills" => scan_skills(dir),
        "tools" => scan_tools(dir),
        "memory" => scan_memory(dir),
        _ => Vec::new(),
    }
}

fn scan_prompts(dir: &Path) -> Vec<String> {
    PROMPT_FILES
        .iter()
        .filter(|stem| dir.join(format!("{stem}.md")).is_file())
        .map(|stem| stem.to_string())
        .collect()
}

fn scan_skills(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        } else if ft.is_dir() && path.join("SKILL.md").is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn scan_tools(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_str() == Some("meta.json") {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

fn scan_memory(dir: &Path) -> Vec<String> {
    if dir.join("memory.md").is_file() {
        vec!["memory".to_string()]
    } else {
        Vec::new()
    }
}

/// Pure snapshot of what `meta.current` points at, per category: resolve
/// each reference → its pool's `current` version → [`scan_resource`]. No
/// card rewrite — used by create/update, which own the card write.
/// Unresolvable references (unknown pool, `current: 0`, missing dir)
/// snapshot as empty / `false`.
pub fn references_snapshot(meta: &AgentMeta) -> AgentReferences {
    AgentReferences {
        prompt_files: resolve_scan(&meta.current.prompt, "prompts"),
        skills: resolve_scan(&meta.current.skills, "skills"),
        tools: resolve_scan(&meta.current.tools, "tools"),
        memory: !resolve_scan(&meta.current.memory, "memory").is_empty(),
    }
}

/// One reference (pool name) → its current version's scan, or empty.
fn resolve_scan(reference: &Option<String>, cat: &str) -> Vec<String> {
    let Some(name) = reference.as_deref() else {
        return Vec::new();
    };
    match resource_current_version_dir(cat, name) {
        Some(dir) => scan_dir(cat, &dir),
        None => Vec::new(),
    }
}

/// Re-scan the card's references and rewrite the `references` block (plus
/// `updated_at`) back into `<name>/meta.json` atomically; returns the new
/// snapshot. The card must exist (`NotFound` otherwise). Used after
/// out-of-band changes to a pool's `current` version so the card's
/// snapshot catches up.
pub fn refresh_agent_references(name: &str) -> io::Result<AgentReferences> {
    validate_agent_name(name).map_err(invalid_input)?;
    let dir = agent_dir(name).ok_or_else(|| not_found("cannot resolve ~/.opencoder"))?;
    let Some(mut meta) = read_agent_meta(name) else {
        return Err(not_found(format!("unknown agent: {name}")));
    };
    let references = references_snapshot(&meta);
    meta.references = references.clone();
    meta.updated_at = now_rfc3339();
    atomic_write_json(&dir.join("meta.json"), &meta)?;
    Ok(references)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scoped;

    fn mkdir_files(root: &Path, rel: &[&str]) {
        for r in rel {
            let p = root.join(r);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
    }

    #[test]
    fn scan_prompts_detects_present_files_in_canonical_order() {
        let (tmp, _g) = scoped();
        mkdir_files(
            tmp.path(),
            &["prompts/pack/v1/soul.md", "prompts/pack/v1/output.md"],
        );
        assert_eq!(scan_resource("prompts", "pack", 1), vec!["soul", "output"]);
        assert!(scan_resource("prompts", "pack", 2).is_empty());
        assert!(scan_resource("prompts", "ghost", 1).is_empty());
    }

    #[test]
    fn scan_skills_dir_and_md_forms() {
        let (tmp, _g) = scoped();
        mkdir_files(
            tmp.path(),
            &[
                "skills/set/v1/alpha/SKILL.md",
                "skills/set/v1/beta.md",
                "skills/set/v1/no_skill_md/other.md",
                "skills/set/v1/notes.txt",
            ],
        );
        // `no_skill_md` lacks SKILL.md, `notes.txt` is not markdown —
        // both are skipped, mirroring skill::discover_in.
        assert_eq!(scan_resource("skills", "set", 1), vec!["alpha", "beta"]);
    }

    #[test]
    fn scan_tools_lists_children_excluding_meta() {
        let (tmp, _g) = scoped();
        mkdir_files(
            tmp.path(),
            &[
                "tools/kit/v1/run.sh",
                "tools/kit/v1/bundle/lib",
                "tools/kit/v1/meta.json",
            ],
        );
        assert_eq!(scan_resource("tools", "kit", 1), vec!["bundle", "run.sh"]);
    }

    #[test]
    fn scan_memory_requires_memory_md() {
        let (tmp, _g) = scoped();
        mkdir_files(tmp.path(), &["memory/bank/v1/memory.md"]);
        assert_eq!(scan_resource("memory", "bank", 1), vec!["memory"]);
        mkdir_files(tmp.path(), &["memory/empty/v1/other.md"]);
        assert!(scan_resource("memory", "empty", 1).is_empty());
    }

    #[test]
    fn refresh_rewrites_snapshot_into_card() {
        let (tmp, _g) = scoped();
        crate::write::save_resource_version(
            "prompts",
            "pack",
            &[crate::write::VersionFile {
                rel_path: "soul.md".into(),
                bytes: b"s".to_vec(),
            }],
        )
        .unwrap();
        crate::write::create_agent("work", Default::default()).unwrap();
        let mut card = read_agent_meta("work").unwrap();
        card.current.prompt = Some("pack".into());
        crate::io::atomic_write_json(&tmp.path().join("work/meta.json"), &card).unwrap();
        // Bump the pool to v2 with one more file, then refresh.
        crate::write::save_resource_version(
            "prompts",
            "pack",
            &[
                crate::write::VersionFile {
                    rel_path: "soul.md".into(),
                    bytes: b"s".to_vec(),
                },
                crate::write::VersionFile {
                    rel_path: "how.md".into(),
                    bytes: b"h".to_vec(),
                },
            ],
        )
        .unwrap();
        let refs = refresh_agent_references("work").unwrap();
        assert_eq!(refs.prompt_files, vec!["soul", "how"]);
        assert_eq!(read_agent_meta("work").unwrap().references, refs);
        assert!(refresh_agent_references("ghost").is_err());
    }
}
