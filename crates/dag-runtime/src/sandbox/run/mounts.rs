use anyhow::{ensure, Context, Result};
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

pub(super) fn overlay(source: &Path, upper: &Path, work: &Path, merged: &Path) -> Result<()> {
    for path in [source, upper, work, merged] {
        ensure!(path.is_absolute(), "overlay paths must be absolute");
        ensure!(
            !path
                .as_os_str()
                .as_bytes()
                .iter()
                .any(|byte| matches!(byte, b',' | b':' | b'\n')),
            "unsupported overlay path"
        );
    }
    real_dir(source)?;
    for path in [upper, work, merged] {
        std::fs::create_dir_all(path)?;
        real_dir(path)?;
    }
    ensure!(
        std::fs::metadata(upper)?.dev() == std::fs::metadata(work)?.dev(),
        "overlay upper and work must use the same filesystem"
    );
    let target = CString::new(merged.as_os_str().as_bytes())?;
    let options = CString::new(format!("lowerdir={},upperdir={},workdir={},index=off,nfs_export=off,metacopy=off,redirect_dir=nofollow", source.display(), upper.display(), work.display()))?;
    let result = unsafe {
        libc::mount(
            c"overlay".as_ptr(),
            target.as_ptr(),
            c"overlay".as_ptr(),
            libc::MS_NODEV | libc::MS_NOSUID,
            options.as_ptr().cast(),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("mount DAG overlay");
    }
    Ok(())
}

pub(super) fn unmount(path: &Path) -> Result<()> {
    let target = CString::new(path.as_os_str().as_bytes())?;
    let result = unsafe { libc::umount2(target.as_ptr(), 0) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOENT)) {
            return Err(error).with_context(|| format!("unmount {}", path.display()));
        }
    }
    Ok(())
}

pub(crate) fn real_dir(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("directory unavailable: {}", path.display()))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "directory must be real: {}",
        path.display()
    );
    let mut current = path.parent();
    while let Some(parent) = current {
        ensure!(
            !std::fs::symlink_metadata(parent)?.file_type().is_symlink(),
            "directory ancestors cannot be symlinks"
        );
        current = parent.parent();
    }
    Ok(())
}

pub(super) fn permissions(path: &Path, uid: u32, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let target = CString::new(path.as_os_str().as_bytes())?;
    ensure!(
        unsafe { libc::chown(target.as_ptr(), uid, 65532) } == 0,
        "set DAG workspace ownership: {}",
        std::io::Error::last_os_error()
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Resolve mountinfo paths beneath the authorized knowledge root, parents first.
/// Duplicate/covered mount records use the currently visible source path once.
pub(super) fn nested_mounts(mountinfo: &str, root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for line in mountinfo.lines() {
        let point = line
            .split_whitespace()
            .nth(4)
            .context("invalid mountinfo record")?;
        let point = PathBuf::from(
            point
                .replace("\\040", " ")
                .replace("\\011", "\t")
                .replace("\\012", "\n")
                .replace("\\134", "\\"),
        );
        ensure!(point.is_absolute(), "mountinfo path must be absolute");
        if point != root && point.starts_with(root) {
            paths.push(point);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_submounts_are_bounded_ordered_and_deduplicated() {
        let mounts = "1 0 0:1 / / rw - ext4 /dev/a rw\n\
2 1 0:2 / /knowledge/nested/child rw - tmpfs tmpfs rw\n\
3 1 0:3 / /knowledge/nested rw - tmpfs tmpfs rw\n\
4 1 0:4 / /knowledge-other/private rw - tmpfs tmpfs rw\n\
5 1 0:5 / /knowledge rw - tmpfs tmpfs rw\n\
6 1 0:6 / /knowledge/nested rw - tmpfs tmpfs rw";
        assert_eq!(
            nested_mounts(mounts, Path::new("/knowledge")).unwrap(),
            vec![
                PathBuf::from("/knowledge/nested"),
                PathBuf::from("/knowledge/nested/child")
            ]
        );
        assert!(nested_mounts(mounts, Path::new("/private"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn knowledge_submount_paths_decode_kernel_escapes_once() {
        let mounts = r"1 0 0:1 / /knowledge/a\040b\011c\012d\134040 rw - tmpfs tmpfs rw";
        assert_eq!(
            nested_mounts(mounts, Path::new("/knowledge")).unwrap(),
            vec![PathBuf::from("/knowledge/a b\tc\nd\\040")]
        );
        assert!(nested_mounts("invalid", Path::new("/knowledge")).is_err());
        assert!(nested_mounts(
            "1 0 0:1 / relative rw - tmpfs tmpfs rw",
            Path::new("/knowledge")
        )
        .is_err());
    }
}
