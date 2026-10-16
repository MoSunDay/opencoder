use anyhow::{ensure, Context, Result};
use opencoder_dag::artifacts::parse_declarations;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
};

fn declarations(dir: &Path) -> Result<Vec<(String, u64, String)>> {
    let mut documents = Vec::new();
    for name in ["artifacts.json", "output.json"] {
        let bytes = match std::fs::read(dir.join(name)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        documents.push((name, serde_json::from_slice(&bytes)?));
    }
    parse_declarations(&documents)
}

pub(super) fn archive(ctx: &super::super::StepCtx) -> Result<()> {
    let target = ctx.dir().map_err(anyhow::Error::msg)?;
    let workspace = super::io::run_root(ctx).join("workspace");
    for (name, expected, digest) in declarations(&target)? {
        let relative = Path::new(&ctx.relative_dir()).join(&name);
        let mut source = super::files::read(&workspace, &relative)?;
        ensure!(
            source.metadata()?.len() == expected,
            "artifact size differs from declaration: {name}"
        );
        let destination = target.join(&name);
        let parent = Path::new(&name)
            .parent()
            .context("artifact parent missing")?;
        if !parent.as_os_str().is_empty() {
            super::files::create_directory(&target, parent)?;
        }
        let staging = destination.with_file_name(format!(".artifact-{}", ulid::Ulid::new()));
        let copied = (|| -> Result<()> {
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            let mut hash = Sha256::new();
            let mut count = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                count = count
                    .checked_add(read as u64)
                    .context("artifact size overflow")?;
                ensure!(count <= expected, "artifact grew during archive: {name}");
                output.write_all(&buffer[..read])?;
                hash.update(&buffer[..read]);
            }
            ensure!(
                count == expected && format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&digest),
                "artifact digest differs from declaration: {name}"
            );
            output.sync_all()?;
            std::fs::rename(&staging, &destination)?;
            std::fs::File::open(
                destination
                    .parent()
                    .context("artifact destination parent missing")?,
            )?
            .sync_all()?;
            Ok(())
        })();
        if copied.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
        copied?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_paths_sizes_and_digests_are_preserved() {
        let digest = "a".repeat(64);
        let documents = [(
            "artifacts.json",
            json!({"files":[{"path":"reports/result.bin","bytes":0,"sha256":digest}]}),
        )];
        assert_eq!(
            parse_declarations(&documents).unwrap(),
            vec![("reports/result.bin".into(), 0, "a".repeat(64))]
        );
        let documents = [(
            "output.json",
            json!({"report_archive":{"file":"report.tar","bytes":42,"sha256":"b".repeat(64)}}),
        )];
        assert_eq!(parse_declarations(&documents).unwrap()[0].1, 42);
    }

    #[test]
    fn malformed_unsafe_reserved_and_duplicate_declarations_fail() {
        for path in [
            "",
            "../host",
            "/host",
            "meta/program",
            "instances/0/output.json",
            "events.ndjson",
            "file\0",
            "a\\b",
        ] {
            let document = json!({"files":[{"path":path,"bytes":1,"sha256":"a".repeat(64)}]});
            assert!(
                parse_declarations(&[("artifacts.json", document)]).is_err(),
                "{path:?}"
            );
        }
        for document in [
            json!({}),
            json!({"files":{}}),
            json!({"files":[{"path":"file","bytes":1,"sha256":"bad"}]}),
        ] {
            assert!(parse_declarations(&[("artifacts.json", document)]).is_err());
        }
        let row = json!({"path":"file","bytes":1,"sha256":"a".repeat(64)});
        assert!(
            parse_declarations(&[("artifacts.json", json!({"files":[row.clone(), row]}))]).is_err()
        );
    }
}
