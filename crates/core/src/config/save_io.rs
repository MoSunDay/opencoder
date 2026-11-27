use std::path::Path;

/// Persist a config save to `target` (body already pretty-printed).
pub(crate) fn write_config_save(target: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let temporary = target.with_extension(format!("tmp-{}", ulid::Ulid::new()));
    let result = (|| {
        let mut file = crate::platform::fs::create_private_file(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        crate::platform::fs::replace(&temporary, target)?;
        crate::platform::fs::sync_directory(target.parent().unwrap_or_else(|| Path::new(".")))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
