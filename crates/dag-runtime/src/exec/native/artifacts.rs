use anyhow::{ensure, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path},
};

fn parse_declarations(documents: &[(&str, Value)]) -> Result<Vec<(String, u64, String)>> {
    let mut declared = Vec::new();
    for (file, value) in documents {
        let rows = if *file == "artifacts.json" {
            value["files"]
                .as_array()
                .context("artifacts.json files must be an array")?
                .clone()
        } else {
            value.get("report_archive").cloned().into_iter().collect()
        };
        for row in rows {
            let name = row
                .get("path")
                .or_else(|| row.get("file"))
                .and_then(Value::as_str)
                .context("artifact path missing")?;
            ensure!(
                !name.is_empty()
                    && !Path::new(name).is_absolute()
                    && !name.contains('\\')
                    && !name.chars().any(char::is_control)
                    && Path::new(name)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "artifact path must be confined"
            );
            let first = Path::new(name)
                .components()
                .next()
                .context("empty artifact path")?;
            ensure!(
                !matches!(
                    first.as_os_str().to_str(),
                    Some(
                        "meta"
                            | "instances"
                            | "meta.json"
                            | "output.txt"
                            | "output.json"
                            | "session.json"
                            | "events.ndjson"
                            | "agent-events"
                            | "transcript.txt"
                            | "artifacts.json"
                    )
                ),
                "artifact path conflicts with step metadata"
            );
            let bytes = row["bytes"].as_u64().context("artifact size missing")?;
            let digest = row["sha256"].as_str().context("artifact digest missing")?;
            ensure!(
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid artifact digest"
            );
            ensure!(
                !declared.iter().any(|(path, _, _)| path == name),
                "duplicate artifact path"
            );
            declared.push((name.into(), bytes, digest.into()));
        }
    }
    Ok(declared)
}

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
            "agent-events/session.ndjson",
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
