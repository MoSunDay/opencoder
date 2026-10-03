use super::{store::check_revision, Store};
use crate::{
    error::AppError,
    text_store::{TextStore, MAX_TEXT_BYTES},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(serde::Serialize)]
pub(crate) struct TextRevisionResult {
    pub revision: i64,
    pub path: String,
    pub format: String,
    pub sha256: String,
    pub bytes: i64,
}

pub(crate) enum TextContent<'a> {
    Inline { content: &'a [u8], format: &'a str },
    NfsPath(&'a str),
    PreparedNfs { path: &'a str, content: &'a [u8] },
}

pub(crate) fn validate_text(content: &[u8], required: bool) -> Result<(), AppError> {
    if content.len() > MAX_TEXT_BYTES {
        return Err(AppError::invalid("text content exceeds 4 MiB"));
    }
    let text = std::str::from_utf8(content).map_err(|_| AppError::invalid("text must be UTF-8"))?;
    if required && text.trim().is_empty() {
        return Err(AppError::invalid("required text cannot be empty"));
    }
    Ok(())
}

impl Store {
    pub(crate) async fn text_revisions(
        &self,
        env: i64,
        entity: Uuid,
        attribute: i64,
    ) -> Result<Vec<Value>, AppError> {
        let mut items = self
            .list::<Value>("text_revisions", env)
            .await?
            .into_iter()
            .filter(|item| {
                item["entity_id"] == entity.to_string()
                    && item["attribute_definition_id"]
                        .as_str()
                        .and_then(|id| id.parse::<i64>().ok())
                        == Some(attribute)
            })
            .collect::<Vec<_>>();
        items.sort_by_key(|item| std::cmp::Reverse(item["revision"].as_i64().unwrap_or(0)));
        Ok(items)
    }

    pub(crate) async fn save_text_revision(
        &self,
        files: &TextStore,
        input: crate::database::model::inputs::TextWrite<'_>,
        actor: &str,
    ) -> Result<TextRevisionResult, AppError> {
        let crate::database::model::inputs::TextWrite {
            env,
            env_key,
            entity,
            attribute,
            input,
            required,
            expected,
        } = input;
        self.entity(env, entity).await?;
        let mut history = self.text_revisions(env, entity, attribute).await?;
        let actual = history
            .iter()
            .find(|item| item["is_current"] == true)
            .and_then(|item| item["revision"].as_i64())
            .unwrap_or(0);
        check_revision(actual, expected)?;
        let revision = actual + 1;
        let (path, format, content, write) = match input {
            TextContent::Inline { content, format } => (
                TextStore::relative_path(env_key, entity, attribute, revision, format)?,
                format.to_string(),
                content.to_vec(),
                true,
            ),
            TextContent::NfsPath(path) => {
                let path = TextStore::nfs_relative_path(path)?;
                let content = files.read_nfs(&path).await?;
                (path, "nfs_path".into(), content, false)
            }
            TextContent::PreparedNfs { path, content } => (
                TextStore::nfs_relative_path(path)?,
                "nfs_path".into(),
                content.to_vec(),
                false,
            ),
        };
        validate_text(&content, required)?;
        let sha256 = hex::encode(Sha256::digest(&content));
        if write {
            files.write_immutable(&path, &content).await?;
        }
        for item in history.iter_mut().filter(|item| item["is_current"] == true) {
            item["is_current"] = json!(false);
            self.put(
                "text_revisions",
                env,
                format!("{entity}/{attribute}/{}", item["revision"]),
                item,
            )
            .await?;
        }
        let path = path.to_string_lossy().to_string();
        let bytes = content.len() as i64;
        let item = json!({"entity_id":entity,"attribute_definition_id":attribute.to_string(),"revision":revision,"content_path":path,"format":format,"sha256":sha256,"bytes":bytes,"status":"ready","is_current":true,"is_deleted":false,"created_by":actor,"created_at":chrono::Utc::now()});
        self.put(
            "text_revisions",
            env,
            format!("{entity}/{attribute}/{revision}"),
            &item,
        )
        .await?;
        self.audit(
            env,
            actor,
            "text.update",
            format!("{entity}/{attribute}"),
            revision,
        )
        .await?;
        Ok(TextRevisionResult {
            revision,
            path,
            format,
            sha256,
            bytes,
        })
    }

    pub(crate) async fn text_revision_path(
        &self,
        env: i64,
        entity: Uuid,
        attribute: i64,
        revision: i64,
    ) -> Result<(String, String), AppError> {
        let item: Value = self
            .get(
                "text_revisions",
                env,
                format!("{entity}/{attribute}/{revision}"),
            )
            .await?;
        Ok((
            item["content_path"]
                .as_str()
                .ok_or(AppError::NotFound)?
                .into(),
            item["format"].as_str().ok_or(AppError::NotFound)?.into(),
        ))
    }

    pub(crate) async fn text_revision_metadata(
        &self,
        env: i64,
        entity: Uuid,
        attribute: i64,
        revision: i64,
    ) -> Result<(String, i64), AppError> {
        let item: Value = self
            .get(
                "text_revisions",
                env,
                format!("{entity}/{attribute}/{revision}"),
            )
            .await?;
        Ok((
            item["sha256"].as_str().ok_or(AppError::NotFound)?.into(),
            item["bytes"].as_i64().ok_or(AppError::NotFound)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_validation_rejects_blank_invalid_and_large_content() {
        assert!(validate_text(b"hello", true).is_ok());
        assert!(validate_text(b"  ", true).is_err());
        assert!(validate_text(&[255], false).is_err());
        assert!(validate_text(&vec![b'a'; MAX_TEXT_BYTES + 1], false).is_err());
        assert!(TextStore::nfs_relative_path("../escape").is_err());
    }
}
