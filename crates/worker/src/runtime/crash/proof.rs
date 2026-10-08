use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Proof {
    pub unit: String,
    pub active_state: String,
    pub control_group: String,
}

pub(super) async fn stopped(unit: &str) -> Result<Proof> {
    ensure!(
        unit.starts_with("opencoder-runtime-")
            && unit.ends_with(".service")
            && unit
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b)),
        "invalid runtime unit"
    );
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("systemctl")
            .args([
                "show",
                unit,
                "--property=LoadState,ActiveState,MainPID,ControlPID,ControlGroup",
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    ensure!(output.status.success(), "cannot inspect runtime unit");
    let text = String::from_utf8(output.stdout)?;
    let fields: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    validate(&fields)?;
    let group = fields
        .get("ControlGroup")
        .context("missing runtime cgroup")?;
    if !group.is_empty() {
        ensure!(
            group.starts_with('/') && !group.split('/').any(|part| part == ".."),
            "invalid runtime cgroup"
        );
        let root = systemd_cgroup_root(Path::new("/sys/fs/cgroup"))?;
        empty_cgroup(&root.join(group.trim_start_matches('/')))?;
    }
    Ok(Proof {
        unit: unit.into(),
        active_state: fields["ActiveState"].into(),
        control_group: (*group).into(),
    })
}

fn systemd_cgroup_root(base: &Path) -> Result<PathBuf> {
    if base.join("cgroup.controllers").is_file() && base.join("cgroup.procs").is_file() {
        return Ok(base.into());
    }
    let legacy = base.join("systemd");
    ensure!(
        legacy.join("cgroup.procs").is_file(),
        "cannot locate the systemd cgroup hierarchy"
    );
    Ok(legacy)
}

fn validate(fields: &BTreeMap<&str, &str>) -> Result<()> {
    ensure!(
        fields.get("LoadState") == Some(&"loaded"),
        "runtime unit must exist"
    );
    ensure!(
        matches!(fields.get("ActiveState"), Some(&"inactive" | &"failed")),
        "runtime unit must be stopped"
    );
    ensure!(
        fields.get("MainPID") == Some(&"0") && fields.get("ControlPID") == Some(&"0"),
        "runtime still owns processes"
    );
    Ok(())
}

fn empty_cgroup(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let processes = std::fs::read_to_string(path.join("cgroup.procs"))?;
    ensure!(
        processes.trim().is_empty(),
        "runtime cgroup still contains processes"
    );
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            empty_cgroup(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_proof_checks_processes_in_both_systemd_cgroup_layouts() {
        for unified in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let base = directory.path();
            assert!(systemd_cgroup_root(base).is_err());
            let root = if unified {
                base.to_path_buf()
            } else {
                base.join("systemd")
            };
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("cgroup.procs"), "").unwrap();
            if unified {
                std::fs::write(root.join("cgroup.controllers"), "memory pids").unwrap();
            }
            assert_eq!(systemd_cgroup_root(base).unwrap(), root);
            let unit = root.join("system.slice/opencoder-runtime-fixture.service");
            std::fs::create_dir_all(unit.join("child")).unwrap();
            std::fs::write(unit.join("cgroup.procs"), "").unwrap();
            std::fs::write(unit.join("child/cgroup.procs"), "42\n").unwrap();
            assert!(empty_cgroup(&unit).is_err());
            std::fs::write(unit.join("child/cgroup.procs"), "").unwrap();
            empty_cgroup(&unit).unwrap();
        }
    }

    #[test]
    fn unknown_active_or_owned_runtime_cannot_release_capacity() {
        let mut fields = BTreeMap::from([
            ("LoadState", "loaded"),
            ("ActiveState", "failed"),
            ("MainPID", "0"),
            ("ControlPID", "0"),
        ]);
        validate(&fields).unwrap();
        for (key, value) in [
            ("LoadState", "not-found"),
            ("ActiveState", "active"),
            ("MainPID", "12"),
            ("ControlPID", "34"),
        ] {
            let old = fields.insert(key, value).unwrap();
            assert!(validate(&fields).is_err());
            fields.insert(key, old);
        }
    }
}
