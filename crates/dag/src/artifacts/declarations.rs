//! Shared, pure validation of declared business artifacts.
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::path::{Component, Path};

pub fn parse_declarations(documents: &[(&str, Value)]) -> Result<Vec<(String, u64, String)>> {
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
                    && name
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
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
            declared.push((name.into(), bytes, digest.to_ascii_lowercase()));
        }
    }
    Ok(declared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declared_files_keep_sizes_and_reject_unsafe_duplicate_or_invalid_entries() {
        let row = json!({"path":"reports/result.txt","bytes":0,"sha256":"A".repeat(64)});
        assert_eq!(
            parse_declarations(&[("artifacts.json", json!({"files":[row.clone()]}))]).unwrap(),
            vec![("reports/result.txt".into(), 0, "a".repeat(64))]
        );
        for path in [
            "../escape",
            "/absolute",
            "a//b",
            "a/./b",
            "meta/key",
            "instances/0/result",
        ] {
            let mut invalid = row.clone();
            invalid["path"] = json!(path);
            assert!(parse_declarations(&[("artifacts.json", json!({"files":[invalid]}))]).is_err());
        }
        assert!(
            parse_declarations(&[("artifacts.json", json!({"files":[row.clone(),row]}))]).is_err()
        );
        assert!(parse_declarations(&[("artifacts.json", json!({"files":null}))]).is_err());
        assert!(parse_declarations(&[(
            "artifacts.json",
            json!({"files":[{"path":"x","bytes":1,"sha256":"bad"}]})
        )])
        .is_err());
    }
}
