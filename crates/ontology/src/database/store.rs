use crate::error::AppError;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use turso::{params, Connection};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Store {
    pub connection: Connection,
    pub include_pending: bool,
}

impl Store {
    pub(crate) fn for_creation(mut self) -> Self {
        self.include_pending = true;
        self
    }

    pub(super) async fn list<T: DeserializeOwned>(
        &self,
        table: &str,
        env: i64,
    ) -> Result<Vec<T>, AppError> {
        let mut rows = self
            .connection
            .query(
                &format!("SELECT body FROM {table} WHERE env_num=?1 ORDER BY id"),
                params![env],
            )
            .await?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().await? {
            values.push(decode(&row.get::<String>(0)?)?);
        }
        Ok(values)
    }

    pub(super) async fn entity_values(
        &self,
        table: &str,
        env: i64,
        entity: Uuid,
    ) -> Result<Vec<Value>, AppError> {
        let mut rows = self
            .connection
            .query(&entity_values_sql(table), params![env, entity.to_string()])
            .await?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await? {
            items.push(decode(&row.get::<String>(0)?)?);
        }
        Ok(items)
    }

    pub(super) async fn get<T: DeserializeOwned>(
        &self,
        table: &str,
        env: i64,
        id: impl ToString,
    ) -> Result<T, AppError> {
        self.optional(table, env, id)
            .await?
            .ok_or(AppError::NotFound)
    }

    pub(super) async fn optional<T: DeserializeOwned>(
        &self,
        table: &str,
        env: i64,
        id: impl ToString,
    ) -> Result<Option<T>, AppError> {
        let mut rows = self
            .connection
            .query(
                &format!("SELECT body FROM {table} WHERE env_num=?1 AND id=?2"),
                params![env, id.to_string()],
            )
            .await?;
        rows.next()
            .await?
            .map(|row| decode(&row.get::<String>(0)?))
            .transpose()
    }

    pub(super) async fn put(
        &self,
        table: &str,
        env: i64,
        id: impl ToString,
        value: &impl Serialize,
    ) -> Result<(), AppError> {
        let body = serde_json::to_string(value)
            .map_err(|error| AppError::dependency(error.to_string()))?;
        self.connection.execute(&format!("INSERT INTO {table}(env_num,id,body) VALUES(?1,?2,?3) ON CONFLICT(env_num,id) DO UPDATE SET body=excluded.body"), params![env, id.to_string(), body]).await?;
        Ok(())
    }

    pub(super) async fn audit(
        &self,
        env: i64,
        actor: &str,
        action: &str,
        subject: impl ToString,
        revision: i64,
    ) -> Result<(), AppError> {
        let subject = subject.to_string();
        let id = digest_id(&format!("{env}/{action}/{subject}/{revision}"));
        self.put("audit_events", env, id, &json!({"actor":actor,"action":action,"subject":subject,"revision":revision,"at":chrono::Utc::now()})).await
    }

    pub(crate) async fn claim_creation(
        &self,
        env: i64,
        request: Uuid,
        fingerprint: &str,
        actor: &str,
    ) -> Result<Uuid, AppError> {
        let id = digest_id(&format!("{env}/{actor}/{request}"));
        if let Some(existing) = self.optional::<Value>("audit_events", env, id).await? {
            if existing["fingerprint"] != fingerprint {
                return Err(AppError::Conflict(
                    "request_id was already used with a different body".into(),
                ));
            }
        } else {
            self.put("audit_events", env, id, &json!({"actor":actor,"request_id":request,"fingerprint":fingerprint,"action":"entity.create.request"})).await?;
        }
        Ok(id)
    }
}

pub(super) fn decode<T: DeserializeOwned>(body: &str) -> Result<T, AppError> {
    serde_json::from_str(body)
        .map_err(|error| AppError::dependency(format!("invalid ontology record: {error}")))
}

pub(super) fn check_revision(actual: i64, expected: i64) -> Result<(), AppError> {
    if actual == expected {
        Ok(())
    } else {
        Err(AppError::Conflict(format!(
            "revision mismatch: expected {expected}, current {actual}"
        )))
    }
}

pub(super) fn numeric_id() -> i64 {
    (i64::from_be_bytes(
        Uuid::new_v4().as_bytes()[..8]
            .try_into()
            .expect("UUID prefix"),
    ) & i64::MAX)
        .max(100)
}

pub(super) fn digest_id(value: &str) -> Uuid {
    Uuid::from_bytes(
        Sha256::digest(value.as_bytes())[..16]
            .try_into()
            .expect("SHA-256 prefix"),
    )
}

pub(super) fn name(value: &str) -> Result<String, AppError> {
    if value.trim().is_empty() {
        Err(AppError::invalid("name is required"))
    } else {
        Ok(value.trim().into())
    }
}

fn entity_values_sql(table: &str) -> String {
    format!("SELECT body FROM {table} INDEXED BY {table}_entity_id WHERE env_num=?1 AND json_extract(body,'$.entity_id')=?2 ORDER BY id")
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn entity_value_queries_use_the_entity_indexes_without_statistics() {
        let temp = tempfile::tempdir().unwrap();
        let database = crate::database::Database::open(
            &temp.path().join("db"),
            &temp.path().join("files"),
            crate::SchemaPolicy::InitializeEmpty,
        )
        .await
        .unwrap();
        let store = database.store().await;
        for table in ["structured_values", "text_revisions"] {
            let mut rows = store
                .connection
                .query(
                    &format!("EXPLAIN QUERY PLAN {}", super::entity_values_sql(table)),
                    turso::params![1, uuid::Uuid::new_v4().to_string()],
                )
                .await
                .unwrap();
            let mut plan = String::new();
            while let Some(row) = rows.next().await.unwrap() {
                plan.push_str(&row.get::<String>(3).unwrap());
            }
            assert!(plan.contains(&format!("{table}_entity_id")), "{plan}");
            let compact: String = plan.chars().filter(|c| !c.is_whitespace()).collect();
            assert!(compact.contains("env_num=?"), "{plan}");
            assert!(
                compact.contains("json_extract(body,'$.entity_id')=?"),
                "{plan}"
            );
        }
    }
}
