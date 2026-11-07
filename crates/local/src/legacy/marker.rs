//! The migration marker (`<source>.migrated.json`): a non-sensitive record
//! of a completed migration (paths, counts, timestamp) used to short-circuit
//! reruns. Reading is strict — an unreadable, non-JSON or wrong-format marker
//! is [`MarkerState::Invalid`] and must abort the run — and writing is atomic
//! (temp file + fsync + rename) so a crash never leaves half a marker.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::rows::MigrationCounts;
use super::MigrateOptions;

/// Marker format understood by this build; anything else is invalid.
const MARKER_FORMAT: u32 = 1;

/// Marker path derived from a source path.
pub fn marker_path(source: &Path) -> PathBuf {
    PathBuf::from(format!("{}.migrated.json", source.display()))
}

/// Non-sensitive record of a completed migration: paths, row counts and the
/// completion timestamp (epoch ms, the store-wide clock). No session content.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MigrationMarker {
    pub format: u32,
    pub source: String,
    pub destination: String,
    pub backup: String,
    pub counts: MigrationCounts,
    pub migrated_at: i64,
}

/// Marker file state. `Invalid` covers unreadable, non-JSON and
/// wrong-format markers; the caller must treat it as a hard error.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkerState {
    Absent,
    Valid(Box<MigrationMarker>),
    Invalid(String),
}

pub fn read_marker(path: &Path) -> Result<MarkerState> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MarkerState::Absent);
        }
        Err(e) => return Ok(MarkerState::Invalid(format!("unreadable: {e}"))),
    };
    match serde_json::from_slice::<MigrationMarker>(&bytes) {
        Ok(marker) if marker.format == MARKER_FORMAT => Ok(MarkerState::Valid(Box::new(marker))),
        Ok(marker) => Ok(MarkerState::Invalid(format!(
            "unsupported marker format {} (expected {MARKER_FORMAT})",
            marker.format
        ))),
        Err(e) => Ok(MarkerState::Invalid(format!("corrupt marker JSON: {e}"))),
    }
}

/// Atomically publish the marker: temp file + fsync + rename + parent fsync,
/// so a crash never leaves a half-written marker behind.
pub fn write_marker(path: &Path, opts: &MigrateOptions, counts: MigrationCounts) -> Result<()> {
    let marker = MigrationMarker {
        format: MARKER_FORMAT,
        source: opts.source.display().to_string(),
        destination: opts.destination.display().to_string(),
        backup: opts.backup.display().to_string(),
        counts,
        migrated_at: opencoder_core::message::now_ms(),
    };
    let body = serde_json::to_vec_pretty(&marker).context("serialize migration marker")?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp).context("create marker temp file")?;
        file.write_all(&body)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path).context("publish migration marker")?;
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::File::open(parent).and_then(|d| d.sync_all())?;
    }
    Ok(())
}

/// What a valid marker means for this run.
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationDecision {
    /// Marker records this exact destination and it exists — nothing to do.
    AlreadyMigrated(MigrationMarker),
    /// Marker exists but its destination is missing — re-migrate and rewrite.
    StaleMarker(MigrationMarker),
    /// No marker; run the migration.
    Proceed,
}

/// Pure decision over an already-validated marker (callers reject
/// [`MarkerState::Invalid`] before asking).
pub fn decide(marker: Option<&MigrationMarker>, destination: &Path) -> MigrationDecision {
    match marker {
        None => MigrationDecision::Proceed,
        Some(m) => {
            if Path::new(&m.destination) == destination && destination.is_dir() {
                MigrationDecision::AlreadyMigrated(m.clone())
            } else {
                MigrationDecision::StaleMarker(m.clone())
            }
        }
    }
}
