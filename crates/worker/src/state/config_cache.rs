//! Stat fingerprint of the config files a node config load reads.
//!
//! `Config::load` stats, parses and merges every candidate file on each
//! call. The node snapshot/report cycle and inventory change checks repeat
//! that work every few seconds, so the effective node config is cached and
//! this module decides cheaply (one active-marker lookup plus file stats, no
//! config parsing or mutation) whether a reload is needed.

use opencoder_core::Config;
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The complete observable state of one covered path. Missing, directories
/// and non-regular file types must not collapse into the same fingerprint:
/// core attempts to read any existing path, so those transitions can change a
/// load from success to failure even when no regular file is present.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileKind {
    Missing,
    Regular { bytes: u64, modified: SystemTime },
    Directory,
    Other,
}

/// One covered path and its file kind/stat metadata. Keeping entries separate
/// avoids hiding a same-size edit to an older file behind a newer file's mtime.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStat {
    path: PathBuf,
    kind: FileKind,
}

/// Stat-derived fingerprint of every path in a covered file set. Equal values
/// mean no covered path changed presence, type, mtime or size.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FileStats {
    entries: Vec<FileStat>,
}

/// Everything a cached config was computed from: where the load reads its
/// files (workdir plus the resolved global/XDG dirs, so callers with a
/// different discovery view never reuse each other's entry), the interactive
/// active-Env selection, and the file stats at those locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigKey {
    workdir: PathBuf,
    global_dir: Option<PathBuf>,
    xdg_dirs: Vec<PathBuf>,
    active_env: Option<PathBuf>,
    stale_marker: bool,
    files: FileStats,
}

/// Candidate files a config load from `workdir` reads. The `config.json`
/// candidate chain comes straight from core discovery
/// (`config::candidates_with_home`), and the domain files come from core's
/// routing table (`config::domain_file_names`), so this set cannot drift
/// from what `Config::load` actually stats and parses. `global_dir` must be
/// core's resolved global config directory (`Config::global_config_path`'s
/// parent); extra `xdg_dirs` cover locations the caller wants fingerprinted
/// even when core's own resolution picks a different one.
pub(crate) fn candidate_files(
    workdir: &Path,
    global_dir: Option<&Path>,
    xdg_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let home = global_dir.and_then(Path::parent);
    let mut paths = opencoder_core::config::candidates_with_home(workdir, home);
    for file in opencoder_core::config::domain_file_names() {
        paths.push(workdir.join(".opencoder").join(file));
        if let Some(dir) = global_dir {
            paths.push(dir.join(file));
        }
    }
    for dir in xdg_dirs {
        for file in ["config.json", "cli.json", "skills.json"] {
            paths.push(dir.join("opencoder").join(file));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Fingerprint every path in `paths`, including missing and non-regular paths.
/// Only `NotFound` means [`FileKind::Missing`]; every other stat or modified-
/// time error is returned so an identical-looking old key cannot hide it.
pub(crate) fn file_stats(paths: &[PathBuf]) -> opencoder_core::Result<FileStats> {
    let mut entries = paths
        .iter()
        .map(|path| {
            let metadata = match std::fs::metadata(path) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(opencoder_core::CoreError::Config(format!(
                        "config input metadata {} failed: {error}",
                        path.display()
                    )));
                }
            };
            let kind = match metadata {
                None => FileKind::Missing,
                Some(metadata) if metadata.is_dir() => FileKind::Directory,
                Some(metadata) if metadata.is_file() => {
                    let modified = metadata.modified().map_err(|error| {
                        opencoder_core::CoreError::Config(format!(
                            "config input modified time {} failed: {error}",
                            path.display()
                        ))
                    })?;
                    FileKind::Regular {
                        bytes: metadata.len(),
                        modified,
                    }
                }
                Some(_) => FileKind::Other,
            };
            Ok(FileStat {
                path: path.clone(),
                kind,
            })
        })
        .collect::<opencoder_core::Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(FileStats { entries })
}

/// Cache key for the interactive `Config::load` from `workdir`. The global dir
/// comes from `Config::global_config_path` so core's own home resolution
/// (including its config-home override) is honored. The XDG locations cover
/// both the platform dir and the override root beside the global dir: core
/// redirects its XDG candidate there under the override, and without it that
/// path is never read — the extra entry only costs one stat.
///
/// Only this interactive key consults `config_env_status`; explicit-home,
/// frozen and operator loaders do not use the worker configuration cache.
pub(crate) fn config_key(workdir: &Path) -> opencoder_core::Result<ConfigKey> {
    let global_dir = Config::global_config_path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let mut xdg_dirs: Vec<PathBuf> = opencoder_core::platform::config_dir().into_iter().collect();
    if let Some(root) = global_dir.as_deref().and_then(Path::parent) {
        xdg_dirs.push(root.to_path_buf());
    }

    // Do not let a marker I/O error reuse a cached no-Env configuration.
    let env = opencoder_core::config_env_status()?;
    let mut candidates = candidate_files(workdir, global_dir.as_deref(), &xdg_dirs);
    // The marker is covered even when stale, and active files are covered even
    // when missing so creation/removal invalidates just like edits do.
    candidates.push(env.marker_path.clone());
    if let Some(directory) = env.active_path.as_ref() {
        for file in ["config.json", "cli.json", "skills.json"] {
            candidates.push(directory.join(file));
        }
    }
    candidates.sort();
    candidates.dedup();
    let files = file_stats(&candidates)?;
    Ok(ConfigKey {
        workdir: workdir.to_path_buf(),
        global_dir,
        xdg_dirs,
        active_env: env.active_path,
        stale_marker: env.stale_marker,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn candidate_files_cover_core_discovery_and_domain_routing() {
        let workdir = Path::new("/work");
        let global_dir = Path::new("/home/.opencoder");
        let xdg_dirs = vec![PathBuf::from("/xdg")];
        // The config.json chain is core's own candidate list for the same
        // home view (project files, global dir, redirected XDG root) ...
        let mut expected: Vec<String> =
            opencoder_core::config::candidates_with_home(workdir, Some(Path::new("/home")))
                .iter()
                .map(|path| path.display().to_string())
                .collect();
        // ... plus every domain file core routes out of config.json ...
        for dir in ["/work/.opencoder", "/home/.opencoder"] {
            for file in opencoder_core::config::domain_file_names() {
                expected.push(format!("{dir}/{file}"));
            }
        }
        // ... plus each extra XDG location the caller covers. XDG supports
        // exactly the same files as an active Env, not MCP or schedules.
        for file in ["config.json", "cli.json", "skills.json"] {
            expected.push(format!("/xdg/opencoder/{file}"));
        }
        let mut actual: Vec<String> = candidate_files(workdir, Some(global_dir), &xdg_dirs)
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn file_stats_stay_equal_until_a_covered_file_changes() {
        let root = tempfile::tempdir().unwrap();
        let workdir = root.path();
        let stats = |paths: &[PathBuf]| file_stats(paths).unwrap();
        let config = workdir.join(".opencoder").join("config.json");
        let candidates = || candidate_files(workdir, None, &[]);

        let empty = stats(&candidates());
        write(&config, "{}");
        let created = stats(&candidates());
        assert_ne!(empty, created, "a created file must change the fingerprint");
        assert_eq!(
            created,
            stats(&candidates()),
            "unchanged files must keep the fingerprint equal"
        );

        write(&config, r#"{"model":"a/much-longer-model-name"}"#);
        let rewritten = stats(&candidates());
        assert_ne!(
            created, rewritten,
            "a rewritten file must change the fingerprint"
        );
    }

    #[test]
    fn file_stats_treat_removed_files_as_missing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencoder.json");
        let before = file_stats(std::slice::from_ref(&path)).unwrap();
        write(&path, "{}");
        assert_ne!(before, file_stats(std::slice::from_ref(&path)).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            before,
            file_stats(std::slice::from_ref(&path)).unwrap(),
            "removal must return to the all-missing fingerprint"
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_stats_distinguish_missing_regular_directory_and_other_types() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let paths = std::slice::from_ref(&path);
        let fingerprint = || file_stats(paths).unwrap();

        let missing = fingerprint();
        assert!(matches!(&missing.entries[0].kind, FileKind::Missing));
        write(&path, "{}");
        let regular = fingerprint();
        assert!(matches!(&regular.entries[0].kind, FileKind::Regular { .. }));
        assert_ne!(missing, regular);

        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let directory = fingerprint();
        assert!(matches!(&directory.entries[0].kind, FileKind::Directory));
        assert_ne!(missing, directory, "a directory is not a missing file");

        std::fs::remove_dir(&path).unwrap();
        let _socket = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let other = fingerprint();
        assert!(matches!(&other.entries[0].kind, FileKind::Other));
        assert_ne!(directory, other);
    }
}
