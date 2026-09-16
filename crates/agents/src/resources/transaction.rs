//! Build immutable versions completely before publishing a resource/card pointer.
use super::filesystem::{check_path, read_files, root, write_files};
use super::model::{merge_files, validate_files, FileEntry, RestoreRequest, SaveRequest};
use super::{card, check_baseline, read_unlocked, set_reference, ResourceView};
use crate::io::{atomic_write_json, invalid_input, now_rfc3339, sync_dir_best_effort};
use opencoder_core::agent::{AgentHistoryEntry, AgentMeta, ResourceMeta};
use std::{fs, io, path::Path};

pub fn save(name: &str, cat: &str, request: SaveRequest) -> io::Result<ResourceView> {
    let _lock = super::lock::write_lock()?;
    let card = card(name)?;
    let meta = check_baseline(&card, cat, &request.baseline)?;
    let original = match &request.baseline.resource {
        Some(resource) => read_files(
            &root()?
                .join(cat)
                .join(resource)
                .join(format!("v{}", request.baseline.version)),
        )?,
        None => vec![],
    };
    let files = merge_files(cat, &original, &request.files, &request.removed)?;
    publish(card, cat, meta, files)?;
    read_unlocked(name, cat)
}

pub fn restore(name: &str, cat: &str, request: RestoreRequest) -> io::Result<ResourceView> {
    let _lock = super::lock::write_lock()?;
    let card = card(name)?;
    let meta = check_baseline(&card, cat, &request.baseline)?
        .ok_or_else(|| invalid_input("resource has no history"))?;
    if !meta.history.contains(&request.version) {
        return Err(invalid_input("version is not in resource history"));
    }
    let resource = request.baseline.resource.as_ref().unwrap();
    let files = read_files(
        &root()?
            .join(cat)
            .join(resource)
            .join(format!("v{}", request.version)),
    )?;
    validate_files(cat, &files)?;
    publish(card, cat, Some(meta), files)?;
    read_unlocked(name, cat)
}

fn next_version(meta: &ResourceMeta) -> io::Result<u32> {
    meta.history
        .iter()
        .copied()
        .chain([meta.current])
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| invalid_input("version number exhausted"))
}

fn publish(
    mut card: AgentMeta,
    cat: &str,
    previous: Option<ResourceMeta>,
    files: Vec<FileEntry>,
) -> io::Result<()> {
    let root = root()?;
    let previous_name = super::reference(&card.current, cat)?.clone();
    let owned = previous
        .as_ref()
        .is_some_and(|m| m.owner_agent.as_deref() == Some(&card.name));
    let resource = if owned {
        previous_name.clone().unwrap()
    } else {
        format!("agent-{}", uuid::Uuid::new_v4().simple())
    };
    let mut meta = previous.clone().unwrap_or_default();
    meta.name = resource.clone();
    meta.owner_agent = Some(card.name.clone());
    let next = next_version(&meta)?;
    let category = root.join(cat);
    check_path(&category)?;
    fs::create_dir_all(&category)?;
    let destination = category.join(&resource);
    check_path(&destination)?;
    // Staging is hidden and excluded from NFS execution snapshots.
    let stage = category.join(format!(".staging~{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&stage)?;
    let result = (|| {
        if !owned {
            if let Some(old) = &previous {
                let source = category.join(previous_name.as_ref().unwrap());
                for version in &old.history {
                    let contents = read_files(&source.join(format!("v{version}")))?;
                    write_files(&stage.join(format!("v{version}")), &contents)?;
                }
            }
        }
        write_files(&stage.join(format!("v{next}")), &files)?;
        let now = now_rfc3339();
        if meta.created_at.is_empty() {
            meta.created_at = now.clone();
        }
        meta.updated_at = now.clone();
        meta.current = next;
        meta.history.push(next);
        if owned {
            let version_dir = destination.join(format!("v{next}"));
            if version_dir.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "version directory already exists",
                ));
            }
            fs::rename(stage.join(format!("v{next}")), &version_dir)?;
            sync_dir_best_effort(&destination);
            if let Err(error) = atomic_write_json(&destination.join("meta.json"), &meta) {
                cleanup(&version_dir);
                return Err(error);
            }
        } else {
            atomic_write_json(&stage.join("meta.json"), &meta)?;
            sync_dir_best_effort(&stage);
            fs::rename(&stage, &destination)?;
            sync_dir_best_effort(&category);
            set_reference(&mut card.current, cat, resource.clone());
            card.updated_at = now.clone();
            card.history.push(AgentHistoryEntry {
                at: now,
                field: if cat == "prompts" { "prompt" } else { cat }.into(),
                from: previous_name,
                to: Some(resource),
            });
            card.references = crate::references::references_snapshot(&card);
            let card_path = root.join(&card.name).join("meta.json");
            if let Err(error) =
                check_path(&card_path).and_then(|_| atomic_write_json(&card_path, &card))
            {
                cleanup(&destination);
                return Err(error);
            }
        }
        Ok(())
    })();
    if stage.exists() {
        cleanup(&stage);
    }
    result
}
fn cleanup(path: &Path) {
    if let Err(error) = fs::remove_dir_all(path) {
        tracing::error!(%error, path = %path.display(), "resource staging cleanup failed");
    }
}
