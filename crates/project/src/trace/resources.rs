//! The actual resource bytes, rather than a mutable current-version pointer.
use anyhow::{Context, Result};
use opencoder_core::agent::{self, Agent};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
fn hash_tree(path: &Path, hash: &mut Sha256) -> Result<()> {
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        anyhow::ensure!(!kind.is_symlink(), "agent resource cannot be a symlink");
        hash.update(entry.file_name().as_encoded_bytes());
        if kind.is_dir() {
            hash_tree(&entry.path(), hash)?;
        } else if kind.is_file() {
            let mut file = std::fs::File::open(entry.path())?;
            let mut bytes = [0u8; 65536];
            loop {
                let n = std::io::Read::read(&mut file, &mut bytes)?;
                if n == 0 {
                    break;
                }
                hash.update(&bytes[..n]);
            }
        }
    }
    Ok(())
}
pub fn identity(agent: &Agent) -> Result<Value> {
    let mut hash = Sha256::new();
    let prompt = opencoder_session::harness::resources::instruction_text(&agent.prompt);
    let harness = opencoder_core::harness::agent_harness(&agent.name);
    hash.update(agent.name.as_bytes());
    hash.update(prompt.as_bytes());
    // Preserve historical native identities while making a Harness switch a
    // new execution identity. Materialized paths never change that identity.
    if harness == opencoder_core::harness::Harness::Codex {
        hash.update(b"\0harness:codex");
    }
    hash.update(serde_json::to_vec(&agent.tools)?);
    let mut versions = serde_json::Map::new();
    if let Some(card) = agent::read_agent_meta(&agent.name) {
        for (category, name) in [
            ("prompts", card.current.prompt),
            ("skills", card.current.skills),
            ("tools", card.current.tools),
            ("memory", card.current.memory),
        ] {
            if let Some(name) = name {
                let meta = agent::read_resource_meta(category, &name)
                    .with_context(|| format!("missing {category}/{name} resource"))?;
                let path = agent::resource_current_version_dir(category, &name)
                    .context("resource version missing")?;
                versions.insert(category.into(), json!({"name":name,"version":meta.current}));
                hash.update(category.as_bytes());
                hash.update(name.as_bytes());
                hash.update(meta.current.to_le_bytes());
                hash_tree(&path, &mut hash)?;
            }
        }
    }
    Ok(
        json!({"name":agent.name,"digest":format!("{:x}",hash.finalize()),"resources":versions,"prompt":prompt,"harness":harness}),
    )
}
