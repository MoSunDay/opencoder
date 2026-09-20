use super::{guest_path, Launch, HOME_MOUNT, LAUNCH_MOUNT};
use crate::sandbox::oci::{self, BundleSpec};
use anyhow::{Context, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

impl Launch {
    /// The login directory is bound directly, outside the artifact mount.
    /// RW is necessary for atomic OAuth refresh and Codex's session/lock files;
    /// copying auth.json would create independently rotating token copies.
    pub fn write_bundle(&self, dir: &Path, spec: &BundleSpec) -> Result<PathBuf> {
        let dir = oci::write_bundle(dir, spec)?;
        let rootfs = dir.join("rootfs");
        let home_mount = guest_path(&rootfs, HOME_MOUNT)?;
        std::fs::create_dir_all(home_mount)?;
        let launch_mount = guest_path(&rootfs, LAUNCH_MOUNT)?;
        std::fs::write(&launch_mount, b"")?;
        let private = dir.join("codex-private");
        std::fs::create_dir_all(&private)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700))?;
        }
        let launch = private.join("launch.json");
        opencoder_core::atomic_write(&launch, &serde_json::to_vec(&self.runtime)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&launch, std::fs::Permissions::from_mode(0o600))?;
        }
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)?;
        let mounts = config["mounts"]
            .as_array_mut()
            .context("OCI mounts missing")?;
        mounts.push(
            json!({"destination":HOME_MOUNT,"type":"bind","source":self.home,
            "options":["rw","rbind","nosuid","nodev"]}),
        );
        mounts.push(
            json!({"destination":LAUNCH_MOUNT,"type":"bind","source":launch,
            "options":["ro","rbind","nosuid","nodev"]}),
        );
        // Secrets stay in the private launch file, not OCI env or DAG artifacts.
        opencoder_core::atomic_write(
            &dir.join("config.json"),
            &serde_json::to_vec_pretty(&config)?,
        )?;
        Ok(dir)
    }
}
