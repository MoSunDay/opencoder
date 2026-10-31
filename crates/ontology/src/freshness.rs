//! Freshness data-plane operations also participate in retirement and cancellation protection.
use crate::{
    database::{Store, REQUEST_CONNECTION},
    domain::{FreshnessRun, MAX_RUN_ITEMS},
    error::AppError,
    http::AppState,
};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum FreshnessOperation {
    /// Active environments able to schedule freshness work.
    Environments,
    PolicyGet(Uuid),
    EntitiesOfType { type_id: Uuid },
    RunCreate { policy_id: Uuid },
    RunGet(Uuid),
    RunSave {
        run: FreshnessRun,
        expected_revision: i64,
    },
    RunsActive,
}

impl AppState {
    /// Read or advance the freshness data plane. Runs are created and updated
    /// exclusively through this entry point; reviews stay behind the HTTP API.
    pub async fn freshness(
        &self,
        env: String,
        operation: FreshnessOperation,
    ) -> Result<Value, AppError> {
        let state = self.clone();
        let active = crate::mutations::Active::new(self.database.clone());
        let write = matches!(
            operation,
            FreshnessOperation::RunCreate { .. } | FreshnessOperation::RunSave { .. }
        );
        tokio::spawn(async move {
            let _active = active;
            let database = state.database.clone();
            let task = move |connection: turso::Connection| async move {
                if write {
                    connection.execute("BEGIN IMMEDIATE", ()).await?;
                }
                let store = Store {
                    connection: connection.clone(),
                    include_pending: false,
                };
                let result = REQUEST_CONNECTION
                    .scope(connection.clone(), async {
                        if let FreshnessOperation::Environments = operation {
                            let items = store
                                .environments(false)
                                .await?
                                .into_iter()
                                .map(|item| json!({"env_key": item.env_key}))
                                .collect::<Vec<_>>();
                            return Ok(json!({"items": items}));
                        }
                        let environment = store.environment(&env).await?;
                        if environment.is_deleted || environment.initialization_status != "ready" {
                            return Err(AppError::NotFound);
                        }
                        let env_num = environment.env_num;
                        match operation {
                            FreshnessOperation::PolicyGet(id) => Ok(
                                json!({"item": store.freshness_policy(env_num, id).await?}),
                            ),
                            FreshnessOperation::EntitiesOfType { type_id } => Ok(json!({"items":
                                store.entities_of_type(env_num, type_id, MAX_RUN_ITEMS as u32).await?
                            })),
                            FreshnessOperation::RunCreate { policy_id } => {
                                let policy = store.freshness_policy(env_num, policy_id).await?;
                                Ok(json!({"item":
                                    store.create_freshness_run(env_num, &policy, "freshness").await?
                                }))
                            }
                            FreshnessOperation::RunGet(id) => Ok(
                                json!({"item": store.freshness_run(env_num, id).await?}),
                            ),
                            FreshnessOperation::RunSave {
                                run,
                                expected_revision,
                            } => Ok(json!({"item":
                                store.save_freshness_run(env_num, run, expected_revision, "freshness").await?
                            })),
                            FreshnessOperation::RunsActive => Ok(json!({"items":
                                store.active_freshness_runs(env_num).await?
                            })),
                            FreshnessOperation::Environments => Ok(Value::Null),
                        }
                    })
                    .await;
                if write {
                    connection
                        .execute(if result.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
                        .await?;
                }
                result.map_err(anyhow::Error::from)
            };
            let result = if write {
                database.db.write(task).await
            } else {
                database.db.read(task).await
            };
            result.map_err(|error| match error.downcast::<AppError>() {
                Ok(error) => error,
                Err(error) => AppError::from(error),
            })
        })
        .await
        .map_err(|error| AppError::dependency(error.to_string()))?
    }
}
