//! Private, immutable interpreter trees: runc may initialize device files
//! before remounting its root read-only, so bundles must not share that root.
use anyhow::{bail, ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

const RUNTIME_DIRS: &[&str] = &["dev", "proc", "sys", "tmp", "workspace/context"];

pub(super) fn snapshot(source: &Path, bundle: &Path) -> Result<PathBuf> {
    let destination = bundle.join("rootfs");
    match fs::symlink_metadata(&destination) {
        Ok(meta) => {
            ensure!(meta.is_dir(), "bundle rootfs must be a real directory");
            return Ok(destination);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        !fs::canonicalize(bundle)?.starts_with(fs::canonicalize(source)?),
        "bundle must be outside the source rootfs"
    );
    let staging = Staging(bundle.join(format!(".rootfs-{}", ulid::Ulid::new())));
    fs::create_dir(&staging.0)?;
    copy_directory(source, &staging.0, Path::new("")).context("copy private runc rootfs")?;
    fs::rename(&staging.0, &destination).context("publish private runc rootfs")?;
    Ok(destination)
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn copy_directory(source: &Path, destination: &Path, relative: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let relative = relative.join(entry.file_name());
        let target = destination.join(entry.file_name());
        if RUNTIME_DIRS.iter().any(|dir| relative == Path::new(dir)) {
            // Never copy live mounts, sockets or devices from a provisioned
            // tree. runc initializes this bundle's own empty device directory.
            fs::create_dir(&target)?;
            continue;
        }
        let kind = entry.file_type()?;
        if relative == Path::new("workspace") {
            ensure!(kind.is_dir(), "rootfs workspace must be a real directory");
        }
        if kind.is_dir() {
            fs::create_dir(&target)?;
            copy_directory(&entry.path(), &target, &relative)?;
        } else if kind.is_file() {
            // Copy bytes, never hard-link shared files that can be updated.
            fs::copy(entry.path(), target)?;
        } else if kind.is_symlink() {
            copy_symlink(&entry.path(), &target)?;
        } else {
            bail!("unsupported rootfs entry: {}", entry.path().display());
        }
    }
    if relative.as_os_str().is_empty() {
        for dir in RUNTIME_DIRS {
            fs::create_dir_all(destination.join(dir))?;
        }
    } else if relative == Path::new("workspace") {
        fs::create_dir_all(destination.join("context"))?;
    }
    fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
    Ok(())
}

fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fs::read_link(source)?, destination)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        bail!("runc rootfs symlinks require Unix")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_isolate_images_devices_and_retries() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("image");
        fs::create_dir_all(source.join("usr/bin")).unwrap();
        fs::create_dir_all(source.join("dev")).unwrap();
        fs::write(source.join("usr/bin/python3"), "version-1").unwrap();
        fs::write(source.join("dev/ptmx"), "old runtime device").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                source.join("usr/bin/python3"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            std::os::unix::fs::symlink("usr/bin", source.join("bin")).unwrap();
        }
        let bundles: Vec<_> = ["a", "b"]
            .map(|name| {
                let bundle = temp.path().join(name);
                fs::create_dir(&bundle).unwrap();
                bundle
            })
            .into();
        let a = snapshot(&source, &bundles[0]).unwrap();
        fs::write(source.join("usr/bin/python3"), "version-2").unwrap();
        let b = snapshot(&source, &bundles[1]).unwrap();
        assert_eq!(
            fs::read_to_string(a.join("usr/bin/python3")).unwrap(),
            "version-1"
        );
        assert_eq!(
            fs::read_to_string(b.join("usr/bin/python3")).unwrap(),
            "version-2"
        );
        assert!(!a.join("dev/ptmx").exists());
        fs::write(a.join("dev/ptmx"), "private runtime device").unwrap();
        assert!(!b.join("dev/ptmx").exists());
        assert_eq!(
            fs::read_to_string(source.join("dev/ptmx")).unwrap(),
            "old runtime device"
        );
        assert_eq!(snapshot(&source, &bundles[0]).unwrap(), a);
        assert_eq!(
            fs::read_to_string(a.join("usr/bin/python3")).unwrap(),
            "version-1"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read_link(a.join("bin")).unwrap(), Path::new("usr/bin"));
            assert_eq!(
                fs::metadata(a.join("usr/bin/python3"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
        assert!(a.join("workspace/context").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_mount_parent_does_not_publish_or_leave_staging() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("image");
        let bundle = temp.path().join("bundle");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&bundle).unwrap();
        std::os::unix::fs::symlink(temp.path(), source.join("workspace")).unwrap();
        assert!(snapshot(&source, &bundle)
            .unwrap_err()
            .to_string()
            .contains("copy private runc rootfs"));
        assert_eq!(fs::read_dir(&bundle).unwrap().count(), 0);
        assert!(!temp.path().join("context").exists());
    }
}
