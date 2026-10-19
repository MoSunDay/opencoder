//! Compare actual current resource content for selection only. Every original
//! target remains resolvable; changing a resource separates its choice again.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

pub(super) fn identity(value: &Value) -> Option<Value> {
    if value["kind"] != "agent" || value.get("unavailable_reason").is_some() {
        return None;
    }
    let name = value["target"].as_str()?;
    let meta = opencoder_core::agent::read_agent_meta(name)?;
    let mut resources = BTreeMap::new();
    for (category, reference) in [
        ("prompts", meta.current.prompt),
        ("skills", meta.current.skills),
        ("tools", meta.current.tools),
        ("memory", meta.current.memory),
    ] {
        if let Some(reference) = reference {
            let root = opencoder_core::agent::resource_current_version_dir(category, &reference)?;
            let mut files = BTreeMap::new();
            collect(&root, &root, &mut files)?;
            resources.insert(category, files);
        }
    }
    let mut definition = value["definition"].clone();
    definition.as_object_mut()?.remove("name");
    Some(json!({"definition":definition,"harness":meta.harness,
        "profile":meta.harness_profile,"resources":resources}))
}

fn collect(root: &Path, path: &Path, files: &mut BTreeMap<String, String>) -> Option<()> {
    let kind = std::fs::symlink_metadata(path).ok()?.file_type();
    if kind.is_symlink() {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?.to_str()?;
    if kind.is_dir() {
        files.insert(format!("{relative}/"), String::new());
        for entry in std::fs::read_dir(path).ok()? {
            collect(root, &entry.ok()?.path(), files)?;
        }
    } else if kind.is_file() {
        let mut file = std::fs::File::open(path).ok()?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let size = file.read(&mut buffer).ok()?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        files.insert(relative.to_owned(), format!("{:x}", hash.finalize()));
    } else {
        return None;
    }
    Some(())
}
