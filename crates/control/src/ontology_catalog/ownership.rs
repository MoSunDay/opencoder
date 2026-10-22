//! Shared catalog writes belong to the active Server activation only.
use crate::release::Lifecycle;
use opencoder_ontology::AppError;
use serde::Deserialize;
use std::{collections::HashMap, fs::File, time::Duration};

#[cfg(all(test, unix))]
#[path = "ownership_tests.rs"]
mod tests;

#[derive(Deserialize)]
struct Journal {
    schema_version: u32,
    current: Option<String>,
    phase: String,
    releases: HashMap<String, Release>,
}

#[derive(Deserialize)]
struct Release {
    server_port: u16,
}

/// Hold this guard through Start, source reads and the final Apply/Error write.
/// Journal.save takes the exclusive side before replacing release-state.json.
/// A missing lock/journal fails closed; standalone Servers need neither file.
pub(super) async fn writer(lifecycle: &Lifecycle) -> Result<Option<File>, AppError> {
    let Some(platform) = lifecycle.platform.get() else {
        return Ok(None);
    };
    let path = platform.state_dir.join("catalog-writer.lock");
    let file = tokio::task::spawn_blocking(move || File::open(path))
        .await
        .map_err(|error| AppError::dependency(error.to_string()))?
        .map_err(|error| AppError::dependency(error.to_string()))?;
    loop {
        match fs2::FileExt::try_lock_shared(&file) {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::select! {
                    _ = lifecycle.retired() => return Err(AppError::dependency("Server is retiring")),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                }
            }
            Err(error) => return Err(AppError::dependency(error.to_string())),
        }
    }
    let bytes = tokio::fs::read(platform.state_dir.join("release-state.json"))
        .await
        .map_err(|error| AppError::dependency(error.to_string()))?;
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|error| AppError::dependency(error.to_string()))?;
    let port = journal
        .releases
        .get(&platform.release_id)
        .map(|r| r.server_port);
    if journal.schema_version != 1
        || journal.current.as_deref() != Some(platform.release_id.as_str())
        || port.is_none()
        || port.as_ref() != lifecycle.listener_port.get()
        || !matches!(
            journal.phase.as_str(),
            "validated" | "warming" | "ready" | "failed" | "verifying" | "complete" | "rolled_back"
        )
    {
        return Err(AppError::dependency(
            "Only the active Server activation may synchronize the capability catalog",
        ));
    }
    Ok(Some(file))
}
