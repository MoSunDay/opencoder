use super::{store::check_revision, Store};
use crate::{domain::validate_vector, error::AppError};
use libsql::params;
use serde_json::{json, Value};
use uuid::Uuid;

impl Store {
    pub(crate) async fn set_vector(
        &self,
        env: i64,
        entity: Uuid,
        attribute: i64,
        input: crate::database::model::inputs::VectorWrite<'_>,
        actor: &str,
    ) -> Result<Uuid, AppError> {
        let crate::database::model::inputs::VectorWrite {
            requested,
            values,
            deleted,
            expected,
        } = input;
        validate_vector(values)?;
        let mut rows = self.connection.query("SELECT vector_id,revision FROM vectors WHERE env_num=?1 AND attribute_id=?2 AND entity_id=?3", params![env,attribute.to_string(),entity.to_string()]).await?;
        let current = rows.next().await?;
        let revision = current
            .as_ref()
            .map(|row| row.get::<i64>(1))
            .transpose()?
            .unwrap_or(0);
        check_revision(revision, expected)?;
        let stored = current
            .map(|row| row.get::<String>(0))
            .transpose()?
            .map(|id| Uuid::parse_str(&id).map_err(|_| AppError::dependency("invalid vector ID")))
            .transpose()?;
        if requested.is_some() && stored.is_some() && requested != stored {
            return Err(AppError::Conflict(
                "vector_id does not match this entity attribute".into(),
            ));
        }
        let id = stored.or(requested).unwrap_or_else(Uuid::new_v4);
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        self.connection.execute("INSERT INTO vectors VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(env_num,attribute_id,entity_id) DO UPDATE SET revision=excluded.revision,is_deleted=excluded.is_deleted,emb=excluded.emb", params![env,attribute.to_string(),entity.to_string(),id.to_string(),revision+1,deleted as i64,bytes]).await?;
        self.audit(env, actor, "vector.update", id, revision + 1)
            .await?;
        Ok(id)
    }

    pub(crate) async fn vector(&self, env: i64, id: Uuid) -> Result<Value, AppError> {
        let mut rows = self.connection.query("SELECT attribute_id,entity_id,emb,revision,is_deleted FROM vectors WHERE env_num=?1 AND vector_id=?2", params![env,id.to_string()]).await?;
        let row = rows.next().await?.ok_or(AppError::NotFound)?;
        let bytes = row.get::<Vec<u8>>(2)?;
        let values = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect::<Vec<_>>();
        Ok(
            json!({"attribute_definition_id":row.get::<String>(0)?,"vector_id":id,"entity_id":row.get::<String>(1)?,"vector":values,"revision":row.get::<i64>(3)?,"is_deleted":row.get::<i64>(4)? != 0}),
        )
    }

    pub(crate) async fn search_vectors(
        &self,
        env: i64,
        attribute: i64,
        values: &[f32],
        limit: u32,
    ) -> Result<Vec<Value>, AppError> {
        validate_vector(values)?;
        let bytes = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut rows = self.connection.query("SELECT vector_id,entity_id,revision,1-vector_distance_cos(emb,vector32(?1)) AS similarity FROM vectors WHERE env_num=?2 AND attribute_id=?3 AND NOT is_deleted ORDER BY similarity DESC,vector_id LIMIT ?4", params![bytes,env,attribute.to_string(),limit as i64]).await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(json!({"vector_id":row.get::<String>(0)?,"entity_id":row.get::<String>(1)?,"revision":row.get::<i64>(2)?,"similarity":row.get::<f64>(3)?}));
        }
        Ok(items)
    }
}
