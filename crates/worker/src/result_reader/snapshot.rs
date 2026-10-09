use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(PartialEq, Eq)]
pub(super) struct Stamp {
    size: u64,
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl Stamp {
    pub fn of(metadata: &Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        }
    }
}

pub(super) struct Snapshot {
    file: File,
    stamp: Stamp,
    pub version: String,
    pub size: u64,
}
impl Snapshot {
    pub fn json(value: &serde_json::Value) -> Result<Self> {
        let mut file = tempfile::tempfile()?;
        {
            let mut writer = std::io::BufWriter::new(&mut file);
            serde_json::to_writer(&mut writer, value)?;
            writer.flush()?;
        }
        Self::file(file)
    }
    pub fn file(mut file: File) -> Result<Self> {
        let stamp = Stamp::of(&file.metadata()?);
        file.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = [0; opencoder_core::fleet::EVENT_CHUNK_BYTES];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        let size = stamp.size;
        Ok(Self {
            file,
            stamp,
            version: format!("{:x}", digest.finalize()),
            size,
        })
    }
    pub fn unchanged(&self) -> Result<bool> {
        Ok(Stamp::of(&self.file.metadata()?) == self.stamp)
    }
    pub fn chunk(&mut self, offset: u64) -> Result<Vec<u8>> {
        ensure!(offset <= self.size, "detail field offset exceeds size");
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![
            0;
            (self.size - offset).min(opencoder_core::fleet::EVENT_CHUNK_BYTES as u64)
                as usize
        ];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

pub(super) fn check_path(path: &Path) -> Result<()> {
    let mut cursor = std::path::PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        ensure!(
            !opencoder_core::platform::fs::is_link(&std::fs::symlink_metadata(&cursor)?),
            "result path contains a symlink"
        );
    }
    Ok(())
}
