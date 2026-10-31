//! Data freshness plane storage: policies, run snapshots and review outcomes.
use super::{
    store::{check_revision, decode, name},
    Store,
};
use crate::{
    domain::{
        recompute_stats, run_status_allows_next, settle_status, truncate_candidate,
        validate_policy_fields,
        Entity, FreshnessExecutor, FreshnessItemStatus, FreshnessPolicy, FreshnessRun,
        FreshnessRunItem, FreshnessRunStatus, MAX_RUN_ITEMS,
    },
    error::AppError,
};
use turso::params;
use uuid::Uuid;

/// Field set shared by policy create and update paths.
pub(crate) struct FreshnessPolicyFields<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub entity_type_id: Uuid,
    pub executor: FreshnessExecutor,
    pub concurrency: u32,
    pub writeback_attribute_key: Option<&'a str>,
}

fn optional_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

impl Store {
    pub(crate) async fn freshness_policies(
        &self,
        env: i64,
        deleted: bool,
    ) -> Result<Vec<FreshnessPolicy>, AppError> {
        Ok(self
            .list::<FreshnessPolicy>("freshness_policies", env)
            .await?
            .into_iter()
            .filter(|item| deleted || !item.is_deleted)
            .collect())
    }

    pub(crate) async fn freshness_policy(
        &self,
        env: i64,
        id: Uuid,
    ) -> Result<FreshnessPolicy, AppError> {
        self.get("freshness_policies", env, id).await
    }

    pub(crate) async fn create_freshness_policy(
        &self,
        env: i64,
        fields: FreshnessPolicyFields<'_>,
        actor: &str,
    ) -> Result<FreshnessPolicy, AppError> {
        validate_policy_fields(
            fields.name,
            fields.entity_type_id,
            &fields.executor,
            fields.concurrency,
            fields.writeback_attribute_key,
        )?;
        if !self
            .entity_types(env, false)
            .await?
            .iter()
            .any(|item| item.id == fields.entity_type_id)
        {
            return Err(AppError::NotFound);
        }
        let item = FreshnessPolicy {
            id: Uuid::new_v4(),
            env_num: env,
            entity_type_id: fields.entity_type_id,
            name: name(fields.name)?,
            description: fields.description.trim().into(),
            executor: fields.executor,
            concurrency: fields.concurrency,
            writeback_attribute_key: optional_key(fields.writeback_attribute_key),
            revision: 0,
            is_deleted: false,
        };
        self.put("freshness_policies", env, item.id, &item).await?;
        self.audit(env, actor, "freshness.policy.create", item.id, 0)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_freshness_policy(
        &self,
        env: i64,
        id: Uuid,
        fields: FreshnessPolicyFields<'_>,
        deleted: bool,
        expected_revision: i64,
        actor: &str,
    ) -> Result<FreshnessPolicy, AppError> {
        validate_policy_fields(
            fields.name,
            fields.entity_type_id,
            &fields.executor,
            fields.concurrency,
            fields.writeback_attribute_key,
        )?;
        let mut item = self.freshness_policy(env, id).await?;
        check_revision(item.revision, expected_revision)?;
        if item.entity_type_id != fields.entity_type_id
            && !self
                .entity_types(env, false)
                .await?
                .iter()
                .any(|kind| kind.id == fields.entity_type_id)
        {
            return Err(AppError::NotFound);
        }
        item.entity_type_id = fields.entity_type_id;
        item.name = name(fields.name)?;
        item.description = fields.description.trim().into();
        item.executor = fields.executor;
        item.concurrency = fields.concurrency;
        item.writeback_attribute_key = optional_key(fields.writeback_attribute_key);
        item.is_deleted = deleted;
        item.revision += 1;
        self.put("freshness_policies", env, id, &item).await?;
        self.audit(env, actor, "freshness.policy.update", id, item.revision)
            .await?;
        Ok(item)
    }

    pub(crate) async fn entities_of_type(
        &self,
        env: i64,
        type_id: Uuid,
        limit: u32,
    ) -> Result<Vec<Entity>, AppError> {
        let mut rows = self
            .connection
            .query(
                "SELECT body FROM entities WHERE env_num=?1 AND json_extract(body,'$.revision')>0 AND NOT json_extract(body,'$.is_deleted') AND json_extract(body,'$.entity_type_id')=?2 ORDER BY id LIMIT ?3",
                params![env, type_id.to_string(), limit as i64],
            )
            .await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(decode(&row.get::<String>(0)?)?);
        }
        Ok(items)
    }

    pub(crate) async fn freshness_runs(
        &self,
        env: i64,
        deleted: bool,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<FreshnessRun>, AppError> {
        Ok(self
            .list::<FreshnessRun>("freshness_runs", env)
            .await?
            .into_iter()
            .filter(|item| deleted || !item.is_deleted)
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }

    pub(crate) async fn freshness_run(&self, env: i64, id: Uuid) -> Result<FreshnessRun, AppError> {
        self.get("freshness_runs", env, id).await
    }

    pub(crate) async fn active_freshness_runs(
        &self,
        env: i64,
    ) -> Result<Vec<FreshnessRun>, AppError> {
        Ok(self
            .list::<FreshnessRun>("freshness_runs", env)
            .await?
            .into_iter()
            .filter(|item| {
                !item.is_deleted
                    && matches!(
                        item.status,
                        FreshnessRunStatus::Queued | FreshnessRunStatus::Running
                    )
            })
            .collect())
    }

    pub(crate) async fn create_freshness_run(
        &self,
        env: i64,
        policy: &FreshnessPolicy,
        actor: &str,
    ) -> Result<FreshnessRun, AppError> {
        let items: Vec<FreshnessRunItem> = self
            .entities_of_type(env, policy.entity_type_id, MAX_RUN_ITEMS as u32)
            .await?
            .into_iter()
            .map(|entity| FreshnessRunItem {
                entity_id: entity.id,
                entity_name: entity.name,
                execution_id: None,
                status: FreshnessItemStatus::Pending,
                candidate: None,
                error: None,
            })
            .collect();
        let mut run = FreshnessRun {
            id: Uuid::new_v4(),
            env_num: env,
            policy_id: policy.id,
            status: FreshnessRunStatus::Queued,
            stats: Default::default(),
            items,
            revision: 0,
            is_deleted: false,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        run.stats = recompute_stats(&run);
        self.put("freshness_runs", env, run.id, &run).await?;
        self.audit(env, actor, "freshness.run.create", run.id, 0)
            .await?;
        Ok(run)
    }

    pub(crate) async fn save_freshness_run(
        &self,
        env: i64,
        mut run: FreshnessRun,
        expected_revision: i64,
        actor: &str,
    ) -> Result<FreshnessRun, AppError> {
        let stored = self.freshness_run(env, run.id).await?;
        check_revision(stored.revision, expected_revision)?;
        if !run_status_allows_next(stored.status, run.status) {
            return Err(AppError::invalid(format!(
                "run status cannot move from {:?} to {:?}",
                stored.status, run.status
            )));
        }
        if run.items.len() > MAX_RUN_ITEMS {
            return Err(AppError::invalid(format!(
                "run items exceed the {MAX_RUN_ITEMS} item limit"
            )));
        }
        if matches!(
            run.status,
            FreshnessRunStatus::Succeeded | FreshnessRunStatus::Partial | FreshnessRunStatus::Failed
        ) && run.status != settle_status(&run)
        {
            return Err(AppError::invalid("terminal run status must match its items"));
        }
        let scope = |run: &FreshnessRun| {
            let mut ids: Vec<Uuid> = run.items.iter().map(|item| item.entity_id).collect();
            ids.sort();
            ids
        };
        if scope(&stored) != scope(&run) {
            return Err(AppError::invalid("run entity scope is immutable"));
        }
        for item in &mut run.items {
            if item
                .execution_id
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(AppError::invalid("execution_id cannot be blank"));
            }
            if let Some(candidate) = &mut item.candidate {
                *candidate = truncate_candidate(candidate);
            }
        }
        run.stats = recompute_stats(&run);
        run.updated_at = chrono::Utc::now().to_rfc3339();
        run.revision = stored.revision + 1;
        self.put("freshness_runs", env, run.id, &run).await?;
        self.audit(env, actor, "freshness.run.update", run.id, run.revision)
            .await?;
        Ok(run)
    }
}
