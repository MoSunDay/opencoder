//! Authenticated, immutable image objects in the existing definition store.
use crate::{
    api::{error_400, error_404, error_500, response},
    AppState,
};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use opencoder_core::fleet::RpcReply;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const MAX_BYTES: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    name: String,
    data_url: String,
}

fn decode(url: &str) -> Result<(&str, Vec<u8>)> {
    ensure!(url.len() <= MAX_BYTES * 4 / 3 + 128, "image exceeds 2 MiB");
    let (header, payload) = url.split_once(',').context("expected image data URL")?;
    let mime = header
        .strip_prefix("data:")
        .and_then(|s| s.strip_suffix(";base64"))
        .context("expected base64 image")?;
    let data = STANDARD.decode(payload).context("invalid image base64")?;
    ensure!(
        !data.is_empty() && data.len() <= MAX_BYTES,
        "image exceeds 2 MiB or is empty"
    );
    let valid = match mime {
        "image/png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    ensure!(
        valid,
        "expected PNG, JPEG or WebP content matching its MIME type"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .context("invalid or oversized decoded image")?;
    Ok((mime, data))
}

pub async fn upload(State(state): State<Arc<AppState>>, Json(body): Json<Upload>) -> Response {
    let (mime, bytes) = match decode(&body.data_url) {
        Ok(v) => v,
        Err(e) => return error_400(e.to_string()),
    };
    if body.name.is_empty() || body.name.len() > 255 || body.name.chars().any(char::is_control) {
        return error_400("invalid image name".into());
    }
    let id = format!("image-{}", ulid::Ulid::new());
    let reference = json!({"id":id,"name":body.name,"mime":mime,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))});
    let value = json!({"reference":reference,"data_url":body.data_url});
    match state
        .fleet
        .put_definition("brain_attachment", &id, &value)
        .await
    {
        Ok(()) => response(RpcReply::ok(reference)),
        Err(e) => error_500(e.to_string()),
    }
}

pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match state.fleet.definition("brain_attachment", &id).await {
        Ok(Some(value)) => response(RpcReply::ok(value)),
        Ok(None) => error_404("image not found"),
        Err(e) => error_500(e.to_string()),
    }
}

pub(super) async fn images(state: &Arc<AppState>, problem: &Value) -> Result<Vec<String>> {
    opencoder_core::brain::pc_issue::validate_problem(problem)?;
    let mut images = vec![];
    for reference in problem["images"].as_array().unwrap() {
        let object = state
            .fleet
            .definition("brain_attachment", reference["id"].as_str().unwrap())
            .await?
            .context("problem image not found")?;
        ensure!(
            &object["reference"] == reference,
            "problem image reference changed"
        );
        let url = object["data_url"]
            .as_str()
            .context("problem image content missing")?;
        let (_, bytes) = decode(url)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == reference["sha256"].as_str().unwrap(),
            "problem image integrity mismatch"
        );
        images.push(url.to_owned());
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_content_type_and_oversized_payload() {
        assert!(decode("data:image/svg+xml;base64,PHN2Zz4=").is_err());
        assert!(decode("data:image/png;base64,aGVsbG8=").is_err());
        assert!(decode(&format!(
            "data:image/jpeg;base64,{}",
            "A".repeat(MAX_BYTES * 2)
        ))
        .is_err());
        assert!(decode("data:image/png;base64,iVBORw0KGgo=").is_err());
        assert!(decode("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC").is_ok());
    }
}
