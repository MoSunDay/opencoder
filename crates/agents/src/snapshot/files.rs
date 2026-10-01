use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

fn open(path: &Path) -> Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "resource must be a regular file"
    );
    Ok(file)
}

pub(super) fn read(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open(path)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "resource metadata exceeds size limit"
    );
    Ok(bytes)
}

pub(super) fn copy(source: &Path, destination: &Path) -> Result<u64> {
    let mut input = open(source)?;
    let mut output = File::create(destination)?;
    Ok(std::io::copy(&mut input, &mut output)?)
}

pub fn digest(root: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_directory(root, root, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_directory(root: &Path, directory: &Path, hasher: &mut Sha256) -> Result<()> {
    let kind = std::fs::symlink_metadata(directory)?.file_type();
    ensure!(
        kind.is_dir() && !kind.is_symlink(),
        "resource snapshot must be a real directory"
    );
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("resource path must be UTF-8")?;
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            hasher.update(b"directory");
            hash_directory(root, &path, hasher)?;
        } else {
            ensure!(
                kind.is_file(),
                "resource snapshot contains a nonregular file"
            );
            hasher.update(b"file");
            let mut file = open(&path)?;
            hasher.update(file.metadata()?.len().to_le_bytes());
            let mut buffer = [0u8; 65536];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
        }
    }
    Ok(())
}
