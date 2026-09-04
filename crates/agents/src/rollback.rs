//! Rollback: point a pool resource's `current` back at a historical
//! version — a pointer switch only, never a deletion.

use std::io;

use opencoder_core::agent::{read_resource_meta, resource_version_dir, validate_resource_name};

use crate::io::{atomic_write_json, invalid_input, now_rfc3339};
use crate::write::resource_dir;

/// Switch `<cat>/<name>`'s `current` to `version` and bump `updated_at`.
/// The version must be in the resource's `history` **and** its version
/// dir must still exist (`InvalidInput` otherwise — no guessing at
/// numbers that were never saved, and no resurrecting pruned versions).
/// Version dirs are never deleted by this operation; a later save still
/// takes `max(history ∪ {current}) + 1`, so numbers are never reused.
pub fn rollback_resource(cat: &str, name: &str, version: u32) -> io::Result<()> {
    validate_resource_name(cat, name).map_err(invalid_input)?;
    let dir = resource_dir(cat, name)?;
    let Some(mut meta) = read_resource_meta(cat, name) else {
        return Err(crate::io::not_found(format!(
            "unknown resource: {cat}/{name}"
        )));
    };
    if !meta.history.contains(&version) {
        return Err(invalid_input(format!(
            "版本 v{version} 不在 {cat}/{name} 的历史中"
        )));
    }
    let Some(vdir) = resource_version_dir(cat, name, version) else {
        return Err(invalid_input(format!("非法版本号: v{version}")));
    };
    if !vdir.is_dir() {
        return Err(invalid_input(format!(
            "版本目录缺失: {cat}/{name}/v{version}"
        )));
    }
    meta.current = version;
    meta.updated_at = now_rfc3339();
    atomic_write_json(&dir.join("meta.json"), &meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::scoped;
    use crate::write::{save_resource_version, VersionFile};

    fn vf(rel: &str) -> VersionFile {
        VersionFile {
            rel_path: rel.into(),
            bytes: rel.as_bytes().to_vec(),
        }
    }

    #[test]
    fn rollback_switches_pointer_keeps_versions() {
        let (tmp, _g) = scoped();
        for n in 1..=2 {
            save_resource_version("skills", "set", &[vf(&format!("s{n}.md"))]).unwrap();
        }
        rollback_resource("skills", "set", 1).unwrap();
        let meta = read_resource_meta("skills", "set").unwrap();
        assert_eq!(meta.current, 1);
        assert_eq!(meta.history, vec![1, 2]); // history intact
        assert!(tmp.path().join("skills/set/v2").is_dir()); // dirs intact
                                                            // Rolling to the same version again is a no-op pointer write.
        rollback_resource("skills", "set", 1).unwrap();
        assert_eq!(read_resource_meta("skills", "set").unwrap().current, 1);
    }

    #[test]
    fn rollback_rejects_unknown_and_missing_versions() {
        let (tmp, _g) = scoped();
        save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
        assert_eq!(
            rollback_resource("tools", "kit", 7).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // In history but the dir was pruned out-of-band → rejected.
        save_resource_version("tools", "kit", &[vf("run.sh")]).unwrap();
        std::fs::remove_dir_all(tmp.path().join("tools/kit/v1")).unwrap();
        assert_eq!(
            rollback_resource("tools", "kit", 1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // Unknown resource → NotFound; unknown category → InvalidInput.
        assert_eq!(
            rollback_resource("tools", "ghost", 1).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            rollback_resource("nope", "kit", 1).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // Nothing was mutated by the failed attempts.
        assert_eq!(read_resource_meta("tools", "kit").unwrap().current, 2);
    }
}
