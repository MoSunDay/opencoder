use anyhow::{ensure, Result};
use std::path::Path;

/// Retain the previous attempt before replacing its current artifact pointers.
pub(crate) fn archive(
    root: &Path,
    run: &str,
    step: &str,
    index: Option<usize>,
    attempt: u64,
) -> Result<()> {
    if attempt == 0 {
        return Ok(());
    }
    let dir = opencoder_dag::artifacts::execution_dir(root, run, step, index)
        .map_err(anyhow::Error::msg)?;
    let history = dir.join("attempts").join(attempt.to_string());
    std::fs::create_dir_all(&history)?;
    for name in [
        "meta.json",
        "output.json",
        "output.txt",
        "transcript.txt",
        "events.ndjson",
        "session.json",
        "messages.json",
        "context.json",
    ] {
        let source = if name == "context.json" {
            let meta = opencoder_dag::artifacts::step_dir(root, run, step)
                .map_err(anyhow::Error::msg)?
                .join("meta");
            match index {
                Some(index) => meta.join("instances").join(index.to_string()).join(name),
                None => meta.join(name),
            }
        } else {
            dir.join(name)
        };
        let raw = match std::fs::read(source) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let target = history.join(name);
        if target.exists() {
            ensure!(std::fs::read(target)? == raw, "attempt history changed");
        } else {
            crate::checkpoint::write(&target, &raw)?;
        }
    }
    Ok(())
}
