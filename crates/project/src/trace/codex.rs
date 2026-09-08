//! External agents declare files on disk; the host owns immutable registration.
use super::{archive::Archive, artifacts};
use anyhow::{ensure, Context, Result};
use opencoder_core::harness::Harness;
use opencoder_session::SessionState;
use std::path::{Path, PathBuf};

pub async fn delivery_prompt(
    session: &mut SessionState,
    run_id: &str,
    prompt: String,
) -> Result<String> {
    if session.harness.harness != Harness::Codex {
        return Ok(prompt);
    }
    opencoder_session::harness::prepare(session).await?;
    let file = manifest_path(session, run_id).context("prepared Codex resources missing")?;
    Ok(format!("{prompt}\nTo deliver files for this project run, write a JSON array of paths relative to the working directory to the following file. Use this run's manifest path even if the plan names an earlier run's path. The host will validate and preserve independent copies after this turn. Omit the manifest when there are no file deliverables.\nOPENCODER_DELIVERABLE_MANIFEST={}\n", file.display()))
}

pub fn manifest_path(session: &SessionState, run_id: &str) -> Option<PathBuf> {
    (session.harness.harness == Harness::Codex)
        .then(|| {
            session
                .harness
                .resource_root
                .as_ref()
                .map(|root| root.join(format!("{run_id}-deliverables.json")))
        })
        .flatten()
}

pub fn collect(archive: &Archive, workdir: &Path, manifest: Option<&Path>) -> Result<()> {
    let Some(manifest) = manifest else {
        return Ok(());
    };
    let meta = match std::fs::symlink_metadata(manifest) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "deliverable manifest must be a regular file"
    );
    ensure!(meta.len() <= 65536, "deliverable manifest exceeds 64 KiB");
    let paths: Vec<String> = serde_json::from_slice(&std::fs::read(manifest)?)?;
    ensure!(paths.len() <= 100, "too many project deliverables");
    for path in paths {
        ensure!(
            !Path::new(&path).is_absolute(),
            "deliverable paths must be relative to the working directory"
        );
        artifacts::register(archive, workdir, &path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn declared_files_are_immutable_and_invalid_paths_fail() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let archive =
            Archive::create(root.path().join("archive"), CancellationToken::new()).unwrap();
        let manifest = work.join("deliverables.json");
        std::fs::write(work.join("report.txt"), "original").unwrap();
        std::fs::write(&manifest, r#"["report.txt"]"#).unwrap();
        collect(&archive, &work, Some(&manifest)).unwrap();
        let metadata = std::fs::read_dir(&archive.root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("artifact-")
                    && p.extension().is_some_and(|s| s == "json")
            })
            .unwrap();
        let entry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(metadata).unwrap()).unwrap();
        std::fs::write(work.join("report.txt"), "replaced").unwrap();
        assert_eq!(
            std::fs::read_to_string(archive.root.join(entry["file"].as_str().unwrap())).unwrap(),
            "original"
        );
        std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
        for invalid in [
            r#"["../outside.txt"]"#,
            r#"["/etc/passwd"]"#,
            r#"{"paths":[]}"#,
            "broken-json",
        ] {
            std::fs::write(&manifest, invalid).unwrap();
            assert!(
                collect(&archive, &work, Some(&manifest)).is_err(),
                "{invalid}"
            );
        }
        archive.check().unwrap(); // Invalid agent input does not poison storage health.
    }

    #[cfg(unix)]
    #[test]
    fn oversized_or_symlinked_manifest_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let archive =
            Archive::create(root.path().join("archive"), CancellationToken::new()).unwrap();
        let manifest = root.path().join("manifest.json");
        std::fs::write(&manifest, vec![b' '; 65537]).unwrap();
        assert!(collect(&archive, root.path(), Some(&manifest)).is_err());
        let link = root.path().join("link.json");
        std::os::unix::fs::symlink(&manifest, &link).unwrap();
        assert!(collect(&archive, root.path(), Some(&link)).is_err());
    }
}
