use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub file: String,
    pub sha256: String,
}

pub fn checksum(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn checked_path(root: &Path, name: &str) -> Result<PathBuf> {
    ensure!(!name.is_empty(), "empty Runner artifact name");
    let mut path = root.to_owned();
    for part in Path::new(name).components() {
        let Component::Normal(part) = part else {
            anyhow::bail!("Runner artifact path escapes output directory");
        };
        path.push(part);
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Runner artifact cannot be a symlink"
        );
    }
    ensure!(path.is_file(), "Runner artifact must be a regular file");
    Ok(path)
}

pub fn verify(root: &Path, artifacts: &[Artifact], result_file: &str) -> Result<serde_json::Value> {
    let mut names = std::collections::BTreeSet::new();
    for artifact in artifacts {
        ensure!(names.insert(&artifact.file), "duplicate Runner artifact");
        ensure!(
            checksum(&checked_path(root, &artifact.file)?)? == artifact.sha256,
            "Runner artifact checksum mismatch: {}",
            artifact.file
        );
    }
    ensure!(
        names.contains(&result_file.to_owned()),
        "Runner result missing from artifact manifest"
    );
    Ok(serde_json::from_slice(&std::fs::read(checked_path(
        root,
        result_file,
    )?)?)?)
}

pub fn validate_registration(settings: &opencoder_core::harness::RunnerSettings) -> Result<()> {
    settings.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        settings.workdir.is_dir(),
        "Runner working directory unavailable"
    );
    let exe = Path::new(&settings.command[0]);
    let meta = exe.metadata().context("Runner executable unavailable")?;
    ensure!(meta.is_file(), "Runner executable must be a file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o111 != 0,
            "Runner executable is not executable"
        );
    }
    for (path, expected) in &settings.files {
        ensure!(
            checksum(path)?.eq_ignore_ascii_case(expected),
            "Runner installation checksum mismatch: {}",
            path.display()
        );
    }
    Ok(())
}
