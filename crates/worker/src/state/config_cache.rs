//! Stat fingerprint of the config files a node config load reads.
//!
//! `Config::load` stats, parses and merges every candidate file on each
//! call. The node snapshot/report cycle and inventory change checks repeat
//! that work every few seconds, so the effective node config is cached and
//! this module decides cheaply (stat only, no reads, no mutation) whether a
//! reload is needed.

use opencoder_core::Config;
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Stat-derived fingerprint of the existing files of a covered file set.
/// Equal values mean no covered file was created, removed or rewritten (by
/// mtime or size) since the previous computation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FileStats {
    pub(crate) newest_mtime: Option<SystemTime>,
    pub(crate) total_bytes: u64,
    pub(crate) file_count: u64,
}

/// Everything a cached config was computed from: where the load reads its
/// files (workdir plus the resolved global/XDG dirs, so callers with a
/// different discovery view never reuse each other's entry) and the file
/// stats at that location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigKey {
    workdir: PathBuf,
    global_dir: Option<PathBuf>,
    xdg_dirs: Vec<PathBuf>,
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
        paths.push(dir.join("opencoder").join("config.json"));
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Fingerprint the existing files among `paths`. A missing path contributes
/// nothing, so creating or removing a covered file changes the result.
pub(crate) fn file_stats(paths: &[PathBuf]) -> FileStats {
    let mut stats = FileStats::default();
    for path in paths {
        let Ok(metadata) = std::fs::metadata(path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        stats.file_count += 1;
        stats.total_bytes = stats.total_bytes.saturating_add(metadata.len());
        let modified = metadata.modified().ok();
        stats.newest_mtime = [stats.newest_mtime, modified]
            .into_iter()
            .flatten()
            .max();
    }
    stats
}

/// Cache key for a config load from `workdir`. The global dir comes from
/// `Config::global_config_path` so core's own home resolution (including its
/// config-home override) is honored. The XDG locations cover both the
/// platform dir and the override root beside the global dir: core redirects
/// its XDG candidate there under the override, and without it that path is
/// never read — the extra entry only costs one stat.
pub(crate) fn config_key(workdir: &Path) -> ConfigKey {
    let global_dir = Config::global_config_path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let mut xdg_dirs: Vec<PathBuf> = opencoder_core::platform::config_dir()
        .into_iter()
        .collect();
    if let Some(root) = global_dir.as_deref().and_then(Path::parent) {
        xdg_dirs.push(root.to_path_buf());
    }
    let files = file_stats(&candidate_files(
        workdir,
        global_dir.as_deref(),
        &xdg_dirs,
    ));
    ConfigKey {
        workdir: workdir.to_path_buf(),
        global_dir,
        xdg_dirs,
        files,
    }
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
        let mut expected: Vec<String> = opencoder_core::config::candidates_with_home(
            workdir,
            Some(Path::new("/home")),
        )
        .iter()
        .map(|path| path.display().to_string())
        .collect();
        // ... plus every domain file core routes out of config.json ...
        for dir in ["/work/.opencoder", "/home/.opencoder"] {
            for file in opencoder_core::config::domain_file_names() {
                expected.push(format!("{dir}/{file}"));
            }
        }
        // ... plus each extra XDG location the caller covers.
        expected.push("/xdg/opencoder/config.json".into());
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
        let stats = |paths: &[PathBuf]| file_stats(paths);
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
        assert_ne!(created, rewritten, "a rewritten file must change the fingerprint");
    }

    #[test]
    fn file_stats_treat_removed_files_as_missing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("opencoder.json");
        let before = file_stats(std::slice::from_ref(&path));
        write(&path, "{}");
        assert_ne!(before, file_stats(std::slice::from_ref(&path)));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            before,
            file_stats(std::slice::from_ref(&path)),
            "removal must return to the all-missing fingerprint"
        );
    }
}
