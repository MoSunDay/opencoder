use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};

pub fn read_only_mount(path: &Path) -> Result<()> {
    let path = path.canonicalize().context("DAG NFS source unavailable")?;
    let mounts = std::fs::read_to_string("/proc/self/mountinfo")?;
    ensure!(
        is_read_only_nfs(&mounts, &path),
        "DAG source must be on a read-only NFS mount: {}",
        path.display()
    );
    std::fs::read_dir(path)?;
    Ok(())
}

fn unescape(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn is_read_only_nfs(mounts: &str, path: &Path) -> bool {
    mounts
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let fields: Vec<_> = left.split_whitespace().collect();
            let point = PathBuf::from(unescape(fields.get(4)?));
            if !path.starts_with(&point) {
                return None;
            }
            let options = fields.get(5)?;
            let kind = right.split_whitespace().next()?;
            Some((
                point.components().count(),
                matches!(kind, "nfs" | "nfs4") && options.split(',').any(|option| option == "ro"),
            ))
        })
        .max_by_key(|(depth, _)| *depth)
        .is_some_and(|(_, valid)| valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_mount_is_required_to_be_nfs_and_read_only() {
        let source = "1 0 0:1 / / rw - ext4 /dev/a rw\n2 1 0:2 / /source ro - nfs host:/src ro\n3 2 0:3 / /source/local rw - tmpfs none rw";
        assert!(is_read_only_nfs(source, Path::new("/source/project")));
        assert!(!is_read_only_nfs(
            source,
            Path::new("/source/local/project")
        ));
        assert!(!is_read_only_nfs(source, Path::new("/source-other")));
        assert!(!is_read_only_nfs(
            &source.replace("/source ro", "/source rw"),
            Path::new("/source")
        ));
    }
}
