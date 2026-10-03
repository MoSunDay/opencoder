use super::{
    store::{check_revision, decode, name},
    Store,
};
use crate::{
    domain::{Entity, Environment},
    error::AppError,
};
use libsql::params;
use uuid::Uuid;

impl Store {
    pub(crate) async fn environments(&self, deleted: bool) -> Result<Vec<Environment>, AppError> {
        let mut rows = self
            .connection
            .query("SELECT body FROM environments ORDER BY env_num", ())
            .await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            let item: Environment = decode(&row.get::<String>(0)?)?;
            if deleted || !item.is_deleted {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub(crate) async fn environment(&self, key: &str) -> Result<Environment, AppError> {
        let mut rows = self
            .connection
            .query(
                "SELECT body FROM environments WHERE env_key=?1",
                params![key],
            )
            .await?;
        decode(
            &rows
                .next()
                .await?
                .ok_or(AppError::NotFound)?
                .get::<String>(0)?,
        )
    }

    pub(crate) async fn create_environment(
        &self,
        key: &str,
        title: &str,
        description: &str,
        actor: &str,
    ) -> Result<Environment, AppError> {
        let key = crate::domain::validate_key(key, "ENV key")?;
        if self.environment(&key).await.is_ok() {
            return Err(AppError::Conflict("ENV key already exists".into()));
        }
        let mut rows = self
            .connection
            .query("SELECT COALESCE(MAX(env_num),0)+1 FROM environments", ())
            .await?;
        let env = rows
            .next()
            .await?
            .ok_or(AppError::NotFound)?
            .get::<i64>(0)?;
        let item = Environment {
            env_num: env,
            env_key: key,
            name: name(title)?,
            description: description.trim().into(),
            initialization_status: "ready".into(),
            revision: 1,
            is_deleted: false,
        };
        self.save_environment(&item).await?;
        self.seed_environment(env, actor).await?;
        self.audit(env, actor, "environment.create", &item.env_key, 1)
            .await?;
        Ok(item)
    }

    pub(crate) async fn update_environment(
        &self,
        key: &str,
        title: &str,
        description: &str,
        deleted: bool,
        expected: i64,
        actor: &str,
    ) -> Result<Environment, AppError> {
        let mut item = self.environment(key).await?;
        check_revision(item.revision, expected)?;
        if key == "debug" && deleted {
            return Err(AppError::invalid("debug ENV cannot be deleted"));
        }
        item.name = name(title)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.revision += 1;
        self.save_environment(&item).await?;
        self.audit(
            item.env_num,
            actor,
            "environment.update",
            key,
            item.revision,
        )
        .await?;
        Ok(item)
    }

    async fn save_environment(&self, item: &Environment) -> Result<(), AppError> {
        let body =
            serde_json::to_string(item).map_err(|error| AppError::dependency(error.to_string()))?;
        self.connection.execute("INSERT INTO environments(env_num,env_key,body) VALUES(?1,?2,?3) ON CONFLICT(env_num) DO UPDATE SET body=excluded.body", params![item.env_num, item.env_key.clone(), body]).await?;
        Ok(())
    }

    pub(crate) async fn entities(
        &self,
        env: i64,
        deleted: bool,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Entity>, AppError> {
        let mut rows = self.connection.query("SELECT body FROM entities WHERE env_num=?1 AND json_extract(body,'$.revision')>0 AND (?2 OR NOT json_extract(body,'$.is_deleted')) ORDER BY id LIMIT ?3 OFFSET ?4", params![env, deleted as i64, limit as i64, offset as i64]).await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(decode(&row.get::<String>(0)?)?);
        }
        Ok(items)
    }

    pub(crate) async fn all_entities(&self, env: i64) -> Result<Vec<Entity>, AppError> {
        Ok(self
            .list::<Entity>("entities", env)
            .await?
            .into_iter()
            .filter(|item| !item.is_deleted && item.revision > 0)
            .collect())
    }

    pub(crate) async fn entity(&self, env: i64, id: Uuid) -> Result<Entity, AppError> {
        let item: Entity = self.get("entities", env, id).await?;
        if item.revision == 0 && !self.include_pending {
            Err(AppError::NotFound)
        } else {
            Ok(item)
        }
    }

    pub(crate) async fn create_entity(
        &self,
        env: i64,
        kind: Uuid,
        title: &str,
        description: &str,
        actor: &str,
    ) -> Result<Entity, AppError> {
        let mut item = self
            .stage_entity(env, Uuid::new_v4(), kind, title, description, actor)
            .await?;
        item.revision = 1;
        self.put("entities", env, item.id, &item).await?;
        Ok(item)
    }

    pub(crate) async fn stage_entity(
        &self,
        env: i64,
        id: Uuid,
        kind: Uuid,
        title: &str,
        description: &str,
        actor: &str,
    ) -> Result<Entity, AppError> {
        if !self
            .entity_types(env, false)
            .await?
            .iter()
            .any(|item| item.id == kind)
        {
            return Err(AppError::NotFound);
        }
        let item = Entity {
            id,
            env_num: env,
            entity_type_id: kind,
            name: name(title)?,
            description: description.trim().into(),
            revision: 0,
            is_deleted: false,
        };
        self.put("entities", env, id, &item).await?;
        self.audit(env, actor, "entity.create", id, 1).await?;
        Ok(item)
    }

    pub(crate) async fn publish_entity(
        &self,
        env: i64,
        id: Uuid,
        actor: &str,
    ) -> Result<Entity, AppError> {
        let mut item: Entity = self.get("entities", env, id).await?;
        item.revision = 1;
        self.put("entities", env, id, &item).await?;
        self.audit(env, actor, "entity.publish", id, 1).await?;
        Ok(item)
    }

    pub(crate) async fn update_entity(
        &self,
        env: i64,
        id: Uuid,
        input: crate::database::model::inputs::EntityUpdate<'_>,
        actor: &str,
    ) -> Result<Entity, AppError> {
        let crate::database::model::inputs::EntityUpdate {
            title,
            description,
            deleted,
            expected,
        } = input;
        let mut item = self.entity(env, id).await?;
        check_revision(item.revision, expected)?;
        item.name = name(title)?;
        item.description = description.trim().into();
        item.is_deleted = deleted;
        item.revision += 1;
        self.put("entities", env, id, &item).await?;
        self.audit(env, actor, "entity.update", id, item.revision)
            .await?;
        Ok(item)
    }
}
