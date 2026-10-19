use anyhow::{ensure, Result};
use std::{io::Write, path::Path};

/// Tools must select the engine before opening a live database file.
pub(crate) fn record(path: &Path) -> Result<()> {
    if path == Path::new(":memory:") {
        return Ok(());
    }
    let mut name = path.as_os_str().to_os_string();
    name.push(".engine.json");
    let marker = Path::new(&name);
    ensure!(
        !marker.is_symlink(),
        "database engine marker cannot be a symlink"
    );
    let expected = serde_json::json!({"engine": "turso", "format": 1});
    if marker.exists() {
        let found: serde_json::Value = serde_json::from_slice(&std::fs::read(marker)?)?;
        ensure!(found == expected, "unsupported database engine marker");
        return Ok(());
    }
    // Concurrent processes publish the same immutable marker. A temporary
    // file and atomic link ensure nobody observes half of a JSON document.
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temporary = marker.with_extension(format!("{}.{epoch}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(serde_json::to_string(&expected)?.as_bytes())?;
    file.sync_all()?;
    let published = std::fs::hard_link(&temporary, marker);
    std::fs::remove_file(&temporary)?;
    if let Err(error) = published {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
        return record(path);
    }
    #[cfg(unix)]
    if let Some(parent) = marker.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
