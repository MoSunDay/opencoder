mod apply;
mod managed;
mod objects;
mod read;

use super::store::digest_id;
use crate::error::AppError;
use opencoder_core::ontology::Capability;
use std::collections::BTreeSet;
use uuid::Uuid;

const ACTOR: &str = "platform-catalog";
const METADATA_ATTRIBUTE: i64 = 91;
const SOURCE_ATTRIBUTE: i64 = 92;
const NOTES_ATTRIBUTE: i64 = 93;

fn entity_id(key: &str) -> Uuid {
    digest_id(&format!("platform-catalog/entity/{key}"))
}
fn type_id() -> Uuid {
    digest_id("platform-catalog/type")
}
fn directory_id() -> Uuid {
    digest_id("platform-catalog/directory")
}
fn aspect_id() -> Uuid {
    digest_id("platform-catalog/aspect")
}

fn validate(entries: &[Capability]) -> Result<(), AppError> {
    let mut keys = BTreeSet::new();
    for entry in entries {
        if entry.key.trim().is_empty()
            || entry.name.trim().is_empty()
            || entry.source.trim().is_empty()
            || !keys.insert(&entry.key)
        {
            return Err(AppError::invalid(
                "catalog requires unique nonempty keys, sources and names",
            ));
        }
    }
    Ok(())
}
