use super::super::{StepCtx, StepResult};
use anyhow::Result;
use opencoder_dag::StepOutcome;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) const CONTEXT_MOUNT: &str = "/workspace";
pub(crate) const AGENTS_MOUNT: &str = "/run/opencoder/agents";
pub(crate) const KNOWLEDGE_MOUNT: &str = "/run/opencoder/knowledge";

pub(crate) fn run_root(ctx: &StepCtx) -> PathBuf {
    ctx.workflow_root.join(&ctx.run_id)
}
pub(crate) fn meta_dir(ctx: &StepCtx) -> Result<PathBuf, String> {
    let logical =
        opencoder_dag::artifacts::step_dir(&ctx.workflow_root, &ctx.run_id, &ctx.step.name)?
            .join("meta");
    Ok(match ctx.instance {
        Some(index) => logical.join("instances").join(index.to_string()),
        None => logical,
    })
}
pub(crate) fn guest_meta(ctx: &StepCtx) -> String {
    let base = format!("{}/{}/meta", CONTEXT_MOUNT, ctx.step.name);
    match ctx.instance {
        Some(index) => format!("{base}/instances/{index}"),
        None => base,
    }
}

pub(crate) fn step_env(ctx: &StepCtx) -> Vec<(String, String)> {
    let mut env = vec![
        ("OPENCODER_RUN_ID".into(), ctx.run_id.clone()),
        (
            "OPENCODER_STEP_DIR".into(),
            format!("{}/{}", CONTEXT_MOUNT, ctx.relative_dir()),
        ),
        ("OPENCODER_STEP_META".into(), guest_meta(ctx)),
        (
            "OPENCODER_STEP_CONTEXT".into(),
            format!("{}/context.json", guest_meta(ctx)),
        ),
        (
            "OPENCODER_STEP_IDEMPOTENCY_KEY".into(),
            format!("{}:{}", ctx.run_id, ctx.execution_key()),
        ),
    ];
    if ctx.knowledge_root.is_some() {
        env.push(("OPENCODER_KNOWLEDGE_DIR".into(), KNOWLEDGE_MOUNT.into()));
    }
    env
}

pub(crate) fn write_context_json(ctx: &StepCtx) -> Result<()> {
    let meta = meta_dir(ctx).map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&meta)?;
    opencoder_core::atomic_write(
        &meta.join("context.json"),
        &serde_json::to_vec_pretty(&ctx.context())?,
    )?;
    super::files::create_directory(
        &run_root(ctx).join("workspace"),
        Path::new(&ctx.relative_dir()),
    )?;
    Ok(())
}

pub(crate) fn archive(ctx: &StepCtx, names: &[&'static str]) -> Result<()> {
    let target = ctx.dir().map_err(anyhow::Error::msg)?;
    std::fs::create_dir_all(&target)?;
    for name in names {
        let path = Path::new(&ctx.relative_dir()).join(name);
        match super::files::read_bounded(
            &run_root(ctx).join("workspace"),
            &path,
            crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
        ) {
            Ok(bytes) => opencoder_core::atomic_write(&target.join(name), &bytes)?,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                match std::fs::remove_file(target.join(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error),
        }
    }
    super::artifacts::archive(ctx)?;
    Ok(())
}

pub(crate) fn finish_from_output_json(dir: &Path, text: String) -> StepResult {
    let path = dir.join("output.json");
    let value = match crate::sandbox::output_limit::read_file_bounded(
        &path,
        "output.json",
        crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES,
    ) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => Some(value),
            Err(error) => return error_result(format!("output.json is not valid JSON: {error}")),
        },
        Err(error) if !path.exists() => {
            let _ = error;
            None
        }
        Err(error) => return error_result(format!("cannot read output.json: {error:#}")),
    };
    StepResult {
        outcome: StepOutcome::Done,
        error: None,
        output_text: text,
        output_json: value,
        session_id: None,
    }
}

pub(crate) fn error_result(error: String) -> StepResult {
    StepResult {
        outcome: StepOutcome::Error,
        error: Some(error),
        output_text: String::new(),
        output_json: None,
        session_id: None,
    }
}

pub(crate) fn tail(text: &str, limit: usize) -> String {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}
