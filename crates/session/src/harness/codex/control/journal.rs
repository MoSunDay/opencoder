//! Stable per-input requests survive observer and control-process restarts.
use crate::SessionState;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Request {
    pub schema_version: u32,
    pub action: String,
    pub input_id: String,
    pub parent_input_id: String,
    pub seq: i64,
    pub original: String,
    pub prompt: String,
    pub images: Vec<String>,
    pub local_rejection: Option<String>,
}

pub(super) fn root(session: &SessionState) -> Result<PathBuf> {
    Ok(session
        .harness
        .resource_root
        .as_ref()
        .context("external input snapshot missing")?
        .join("external-inputs"))
}

pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("external input directory missing")?)?;
    let temp = path.with_extension(format!("tmp-{}", ulid::Ulid::new()));
    std::fs::write(&temp, serde_json::to_vec(value)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

pub(super) fn request_path(session: &SessionState, seq: i64) -> Result<PathBuf> {
    Ok(root(session)?.join(format!("{seq}.request.json")))
}

pub(super) fn load(session: &SessionState, seq: i64) -> Result<Option<Request>> {
    let path = request_path(session, seq)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            let request: Request = serde_json::from_slice(&bytes)?;
            ensure!(
                request.seq == seq && request.input_id == format!("external-input-{seq}"),
                "external input journal identity mismatch"
            );
            Ok(Some(request))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) async fn prepare(
    session: &SessionState,
    seq: i64,
    text: &str,
    images: &[String],
) -> Result<Request> {
    if let Some(request) = load(session, seq)? {
        ensure!(
            request.original == text && request.images == images,
            "external input payload changed"
        );
        return Ok(request);
    }
    let rejection = crate::control_cmd::split_control_prefix(text).map(|_| {
        "Control commands require an explicit turn boundary; this input was not delivered"
            .to_string()
    });
    let previous = session.skill_prompt_cloned();
    let mut prompt = if rejection.is_none() {
        crate::skill_resolve::resolve_inline_skills(session, text)
    } else {
        text.to_owned()
    };
    crate::skill_resolve::persist_active_skill(session, &previous).await;
    if prompt.trim().is_empty() && session.skill_prompt_cloned().is_some() {
        prompt = crate::skill_resolve::SKILL_TRIGGER.to_owned();
    }
    if let Some(skill) = session.skill_prompt_cloned() {
        prompt = format!("# Active skill\n{skill}\n\n{prompt}");
    }
    let request = Request {
        schema_version: 1,
        action: "steer".into(),
        input_id: format!("external-input-{seq}"),
        parent_input_id: session
            .harness
            .last_input_id
            .clone()
            .context("external input parent missing")?,
        seq,
        original: text.to_owned(),
        prompt,
        images: images.to_vec(),
        local_rejection: rejection,
    };
    write(&request_path(session, seq)?, &request)?;
    Ok(request)
}
