//! Durable node-local command mailbox. Only the scheduler writes step state.
use anyhow::{ensure, Context, Result};
use opencoder_dag::{
    control::{Command, Receipt, StepStatus},
    DagSpec, RecoveryPolicy, StepKind,
};
use std::path::{Path, PathBuf};

pub(crate) fn directory(root: &Path, run: &str) -> Result<PathBuf> {
    Ok(opencoder_dag::artifacts::run_root(root, run)
        .map_err(anyhow::Error::msg)?
        .join(".step-control"))
}

pub fn status(root: &Path, run: &str) -> Result<Vec<StepStatus>> {
    let file = directory(root, run)?.join("state.json");
    match std::fs::read(file) {
        Ok(raw) => Ok(serde_json::from_slice(&raw)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn receipt(root: &Path, run: &str, operation: &str) -> Result<Option<Receipt>> {
    validate_id(operation)?;
    let dir = directory(root, run)?;
    for name in ["receipts", "requests"] {
        match std::fs::read(dir.join(name).join(format!("{operation}.json"))) {
            Ok(raw) => return Ok(Some(serde_json::from_slice(&raw)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(None)
}

/// Caller serializes submissions for this run using its existing lifecycle gate.
pub fn submit(root: &Path, run: &str, spec: &DagSpec, command: Command) -> Result<Receipt> {
    command.validate().map_err(anyhow::Error::msg)?;
    if let Some(prior) = receipt(root, run, &command.operation_id)? {
        ensure!(
            prior.command == command,
            "operation ID already has different input"
        );
        return Ok(prior);
    }
    let step = spec
        .steps
        .iter()
        .find(|s| s.name == command.step)
        .context("step not found")?;
    ensure!(
        step.recovery_policy == RecoveryPolicy::Operator,
        "step does not allow operator recovery"
    );
    ensure!(
        matches!(step.kind, StepKind::Dynamic { .. }) || command.instance.is_none(),
        "static step cannot have an instance index"
    );
    let states = status(root, run)?;
    let state = states
        .iter()
        .find(|s| s.step == command.step && s.instance == command.instance)
        .context("step is not initialized")?;
    ensure!(
        state.attempt == command.expected_attempt,
        "step attempt changed"
    );
    let receipt = Receipt {
        command,
        phase: "queued".into(),
        error: None,
        updated_at: opencoder_core::message::now_ms(),
    };
    let dir = directory(root, run)?.join("requests");
    std::fs::create_dir_all(&dir)?;
    crate::checkpoint::write(
        &dir.join(format!("{}.json", receipt.command.operation_id)),
        &serde_json::to_vec(&receipt)?,
    )?;
    Ok(receipt)
}

fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
        "invalid operation ID"
    );
    Ok(())
}

pub(crate) fn pending(root: &Path, run: &str) -> Result<Vec<Receipt>> {
    let dir = directory(root, run)?;
    std::fs::create_dir_all(dir.join("requests"))?;
    let mut result = Vec::new();
    for item in std::fs::read_dir(dir.join("requests"))? {
        let path = item?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if dir
            .join("receipts")
            .join(path.file_name().unwrap())
            .exists()
        {
            continue;
        }
        let row: Receipt = serde_json::from_slice(&std::fs::read(path)?)?;
        row.command.validate().map_err(anyhow::Error::msg)?;
        result.push(row);
    }
    result.sort_by(|a, b| {
        (a.updated_at, &a.command.operation_id).cmp(&(b.updated_at, &b.command.operation_id))
    });
    Ok(result)
}

pub(crate) fn finish(
    root: &Path,
    run: &str,
    mut row: Receipt,
    error: Option<String>,
) -> Result<()> {
    row.phase = if error.is_some() {
        "rejected"
    } else {
        "applied"
    }
    .into();
    row.error = error;
    row.updated_at = opencoder_core::message::now_ms();
    let dir = directory(root, run)?.join("receipts");
    std::fs::create_dir_all(&dir)?;
    crate::checkpoint::write(
        &dir.join(format!("{}.json", row.command.operation_id)),
        &serde_json::to_vec(&row)?,
    )
}

pub(crate) fn save(root: &Path, run: &str, states: &[StepStatus]) -> Result<()> {
    let dir = directory(root, run)?;
    std::fs::create_dir_all(&dir)?;
    crate::checkpoint::write(&dir.join("state.json"), &serde_json::to_vec(states)?)
}

mod history;
pub(crate) use history::archive;
