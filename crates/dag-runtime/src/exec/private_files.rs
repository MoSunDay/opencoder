//! Expose only the owning execution's private files, outside public DAG artifacts.
use anyhow::{ensure, Context, Result};
use opencoder_core::fleet::private_files::GUEST_ROOT;
use std::path::Path;

pub(super) fn prompt(prompt: String, root: Option<&Path>) -> String {
    let Some(root) = root else { return prompt };
    format!("{prompt}\n\n执行器私有任务文件目录：{}。只把其中的文件路径传给任务工具；不得将凭证内容读入模型上下文、prompt、日志或最终结果。任务目录只读，结果和临时文件写入当前步骤工作目录。",root.display())
}

pub(super) fn bind(bundle: &Path, root: &Path) -> Result<()> {
    bind_at(bundle, root, GUEST_ROOT)
}

pub(super) fn bind_at(bundle: &Path, root: &Path, guest: &str) -> Result<()> {
    ensure!(root.is_absolute(), "private task root must be absolute");
    let meta = std::fs::symlink_metadata(root).context("private task directory unavailable")?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "private task directory must be real"
    );
    let config_path = bundle.join("config.json");
    let mut config: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    ensure!(
        matches!(guest, GUEST_ROOT | "/run/opencoder-device"),
        "unsupported private mount"
    );
    let target = bundle.join("rootfs").join(guest.trim_start_matches('/'));
    for parent in target.ancestors().take_while(|path| *path != bundle) {
        if let Ok(meta) = std::fs::symlink_metadata(parent) {
            ensure!(
                !meta.file_type().is_symlink(),
                "private mount cannot traverse symlinks"
            );
        }
    }
    std::fs::create_dir_all(target)?;
    let mounts = config["mounts"]
        .as_array_mut()
        .context("OCI mount list missing")?;
    ensure!(
        !mounts.iter().any(|mount| mount["destination"] == guest),
        "duplicate private task mount"
    );
    mounts.push(serde_json::json!({"destination":guest,"type":"bind","source":root,"options":["rbind","ro","nosuid","nodev"]}));
    opencoder_core::atomic_write(&config_path, &serde_json::to_vec_pretty(&config)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_mount_is_read_only_and_prompts_include_paths_only() {
        let temp = tempfile::tempdir().unwrap();
        let bundle = temp.path().join("bundle");
        let private = temp.path().join("private");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("credential"), "fixture-hidden-token").unwrap();
        std::fs::write(bundle.join("config.json"), r#"{"mounts":[]}"#).unwrap();
        bind(&bundle, &private).unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(bundle.join("config.json")).unwrap()).unwrap();
        assert_eq!(config["mounts"][0]["destination"], GUEST_ROOT);
        assert!(config["mounts"][0]["options"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("ro")));
        let text = prompt("run task".into(), Some(&private));
        assert!(text.contains(private.to_str().unwrap()));
        assert!(!text.contains("fixture-hidden-token"));
        assert!(bind(&bundle, &private).is_err());
    }
}
