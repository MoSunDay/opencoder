use crate::agent;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// A content identity for the actual agent card and referenced resource
/// versions. Plans persist the digest, never the resource file contents.
pub fn agent_manifest(name: &str) -> Result<String, String> {
    let resolved =
        agent::resolve_agent(name).ok_or_else(|| format!("agent {name} is unavailable"))?;
    let mut manifest = BTreeMap::new();
    manifest.insert("resolved".to_string(),json!({"name":resolved.name,"kind":resolved.kind,"mode":resolved.mode,"prompt":resolved.prompt,"tools":resolved.tools}).to_string());
    if let Some(meta) = agent::read_agent_meta(name)
        .filter(|_| !agent::builtin_agents().iter().any(|a| a.name == name))
    {
        manifest.insert(
            "card".into(),
            serde_json::to_string(&meta).map_err(|e| e.to_string())?,
        );
        for (category, reference) in [
            ("prompts", meta.current.prompt),
            ("skills", meta.current.skills),
            ("tools", meta.current.tools),
            ("memory", meta.current.memory),
        ] {
            if let Some(reference) = reference {
                let root = agent::resource_current_version_dir(category, &reference)
                    .ok_or_else(|| format!("resource {category}/{reference} unavailable"))?;
                hash_files(
                    &root,
                    &root,
                    &format!("{category}/{reference}"),
                    &mut manifest,
                )?;
            }
        }
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
    ))
}
fn hash_files(
    root: &Path,
    path: &Path,
    prefix: &str,
    manifest: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "resource symlink is unsupported: {}",
                path.display()
            ));
        }
        if kind.is_dir() {
            hash_files(root, &path, prefix, manifest)?;
        } else {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            manifest.insert(
                format!(
                    "{prefix}/{}",
                    path.strip_prefix(root)
                        .map_err(|e| e.to_string())?
                        .display()
                ),
                format!("{:x}", Sha256::digest(bytes)),
            );
        }
    }
    Ok(())
}
