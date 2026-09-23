//! Freeze PC evidence under its execution for authenticated artifact downloads.
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::ArtifactRef, fleet::ExecutionRef};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub fn freeze(
    root: &Path,
    execution: &ExecutionRef,
    evidence: &mut [Value],
) -> Result<Vec<ArtifactRef>> {
    ensure!(evidence.len() <= 128, "too many PC issue evidence files");
    let directory = root.join("pc-evidence");
    std::fs::create_dir_all(&directory)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let mut artifacts = vec![];
    for (index, item) in evidence.iter_mut().enumerate() {
        let path = Path::new(item["path"].as_str().context("evidence path required")?);
        ensure!(path.is_absolute(), "evidence must use an absolute path");
        let mut source =
            std::fs::File::open(path).context("PC evidence unavailable on execution node")?;
        let metadata = source.metadata()?;
        ensure!(
            metadata.is_file() && Some(metadata.len()) == item["bytes"].as_u64(),
            "PC evidence size mismatch"
        );
        let expected = item["sha256"]
            .as_str()
            .context("evidence hash required")?
            .to_owned();
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .filter(|s| s.len() <= 8 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            .unwrap_or("bin");
        let name = format!("{index}-{}.{}", &expected[..16], extension);
        let pending = directory.join(format!("{}.pending", ulid::Ulid::new()));
        let mut target = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&pending)?;
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 65536];
        let mut size = 0;
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            target.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            size += count as u64;
        }
        ensure!(
            format!("{:x}", hash.finalize()) == expected && size == metadata.len(),
            "PC evidence changed or hash mismatched"
        );
        target.sync_all()?;
        std::fs::rename(pending, directory.join(&name))?;
        let reference = ArtifactRef {
            execution: execution.clone(),
            step: "pc-evidence".into(),
            file: name,
            sha256: expected,
            bytes: size,
        };
        item["artifact"] = serde_json::to_value(&reference)?;
        artifacts.push(reference);
    }
    std::fs::File::open(directory)?.sync_all()?;
    Ok(artifacts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn snapshots_bytes_and_rejects_false_hash() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.png");
        std::fs::write(&source, b"evidence").unwrap();
        let execution = ExecutionRef {
            kind: opencoder_core::fleet::ExecutionKind::Operator,
            id: "operator-pc-test".into(),
        };
        let mut items = vec![
            json!({"path":source,"bytes":8,"sha256":format!("{:x}",Sha256::digest(b"evidence"))}),
        ];
        let artifacts = freeze(dir.path(), &execution, &mut items).unwrap();
        std::fs::write(&source, b"modified").unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("pc-evidence").join(&artifacts[0].file)).unwrap(),
            b"evidence"
        );
        assert!(freeze(dir.path(), &execution, &mut items).is_err());
    }
}
