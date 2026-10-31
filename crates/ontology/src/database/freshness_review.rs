//! Freshness review resolution: duplicate checks, approvals and writeback.
use super::store::{digest_id, Store};
use crate::{
    database::{model::inputs::TextWrite, text_revision::TextContent},
    domain::{
        truncate_candidate, AttributeKind, FreshnessItemStatus, FreshnessReview,
        FreshnessVerdict, StorageMode,
    },
    error::AppError,
    text_store::TextStore,
};
use serde_json::{json, Value};
use uuid::Uuid;

impl Store {
    pub(crate) async fn freshness_reviews(
        &self,
        env: i64,
        run: Option<Uuid>,
    ) -> Result<Vec<FreshnessReview>, AppError> {
        Ok(self
            .list::<FreshnessReview>("freshness_reviews", env)
            .await?
            .into_iter()
            .filter(|item| run.is_none_or(|id| item.run_id == id))
            .collect())
    }

    pub(crate) async fn freshness_review(
        &self,
        env: i64,
        run: Uuid,
        entity: Uuid,
    ) -> Result<Option<FreshnessReview>, AppError> {
        Ok(self
            .freshness_reviews(env, Some(run))
            .await?
            .into_iter()
            .find(|item| item.entity_id == entity))
    }

    /// Record one review decision; approval additionally promotes the entity
    /// data level and optionally writes the candidate back to a text attribute.
    /// Callers wrap this in a write transaction.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) async fn create_freshness_review(
        &self,
        env: i64,
        run_id: Uuid,
        entity_id: Uuid,
        verdict: FreshnessVerdict,
        comment: &str,
        files: &TextStore,
        actor: &str,
    ) -> Result<Value, AppError> {
        let run = self.freshness_run(env, run_id).await?;
        if run.is_deleted {
            return Err(AppError::NotFound);
        }
        let policy = self.freshness_policy(env, run.policy_id).await?;
        let item = run
            .items
            .iter()
            .find(|item| item.entity_id == entity_id)
            .ok_or(AppError::NotFound)?;
        if self.freshness_review(env, run_id, entity_id).await?.is_some() {
            return Err(AppError::Conflict("item already reviewed".into()));
        }
        let mut entity = self.entity(env, entity_id).await?;
        if entity.is_deleted {
            return Err(AppError::Conflict("entity is deleted".into()));
        }
        let from_level = entity.data_level;
        let mut to_level = from_level;
        let mut text_revision: Option<Value> = None;
        if verdict == FreshnessVerdict::Approve {
            let candidate = item
                .candidate
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            if item.status != FreshnessItemStatus::Succeeded || candidate.is_none() {
                return Err(AppError::Conflict("no candidate to approve".into()));
            }
            let candidate = truncate_candidate(candidate.expect("presence checked above"));
            self.catalog_assert_editable(env, "entities", &entity_id.to_string())
                .await?;
            if let Some(key) = policy.writeback_attribute_key.as_deref() {
                let definition = self
                    .attributes(env, Some(entity.entity_type_id), false)
                    .await?
                    .into_iter()
                    .find(|item| item.attribute_key == key)
                    .ok_or(AppError::NotFound)?;
                if definition.kind != AttributeKind::Text
                    || definition.storage_mode != StorageMode::Markdown
                {
                    return Err(AppError::invalid(
                        "writeback attribute must store markdown text",
                    ));
                }
                self.catalog_assert_editable(
                    env,
                    "attribute_definitions",
                    &definition.id.to_string(),
                )
                .await?;
                let current = self
                    .text_revisions(env, entity_id, definition.id)
                    .await?
                    .iter()
                    .find(|item| item["is_current"] == true)
                    .and_then(|item| item["revision"].as_i64())
                    .unwrap_or(0);
                let result = self
                    .save_text_revision(
                        files,
                        TextWrite {
                            env,
                            env_key: &self.environment_key(env).await?,
                            entity: entity_id,
                            attribute: definition.id,
                            input: TextContent::Inline {
                                content: candidate.as_bytes(),
                                format: "md",
                            },
                            required: definition.required,
                            expected: current,
                        },
                        actor,
                    )
                    .await?;
                text_revision = Some(
                    serde_json::to_value(result)
                        .map_err(|_| AppError::dependency("invalid text revision"))?,
                );
            }
            to_level = from_level.next().unwrap_or(from_level);
            entity.data_level = to_level;
            entity.revision += 1;
            self.put("entities", env, entity.id, &entity).await?;
            self.audit(env, actor, "entity.data_level", entity.id, entity.revision)
                .await?;
        }
        let review = FreshnessReview {
            id: digest_id(&format!("{env}/freshness-review/{run_id}/{entity_id}")),
            env_num: env,
            run_id,
            policy_id: run.policy_id,
            entity_id,
            from_level,
            to_level,
            verdict,
            comment: comment.trim().into(),
            actor: actor.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.put("freshness_reviews", env, review.id, &review).await?;
        self.audit(
            env,
            actor,
            "freshness.review.create",
            format!("{run_id}/{entity_id}"),
            run.revision,
        )
        .await?;
        Ok(json!({
            "item": review,
            "entity": {"id": entity_id, "data_level": to_level, "revision": entity.revision},
            "text_revision": text_revision,
        }))
    }

    async fn environment_key(&self, env: i64) -> Result<String, AppError> {
        self.environments(false)
            .await?
            .into_iter()
            .find(|item| item.env_num == env)
            .map(|item| item.env_key)
            .ok_or(AppError::NotFound)
    }
}
