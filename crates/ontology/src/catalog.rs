//! Leased catalog operations also participate in retirement and cancellation protection.
use crate::{
    database::{Store, REQUEST_CONNECTION},
    error::AppError,
    http::AppState,
};
use opencoder_core::ontology::Capability;
use serde_json::Value;

#[derive(Clone, Debug)]
pub enum CatalogOperation {
    List,
    Get(String),
    Status,
    Start,
    Check(Vec<Capability>),
    Apply(Vec<Capability>),
    Error(String),
}

impl AppState {
    /// Read or reconcile a complete source snapshot. Callers serialize source reads
    /// and Apply across processes; failed source reads must use Error, never Apply([]).
    pub async fn catalog(
        &self,
        env: String,
        operation: CatalogOperation,
    ) -> Result<Value, AppError> {
        let state = self.clone();
        let active = crate::mutations::Active::new(self.database.clone());
        let write = matches!(
            operation,
            CatalogOperation::Start | CatalogOperation::Apply(_) | CatalogOperation::Error(_)
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
                        let environment = store.environment(&env).await?;
                        if environment.is_deleted || environment.initialization_status != "ready" {
                            return Err(AppError::NotFound);
                        }
                        let env_num = environment.env_num;
                        match operation {
                            CatalogOperation::List => store.catalog_list(env_num).await,
                            CatalogOperation::Get(id) => store.catalog_get(env_num, &id).await,
                            CatalogOperation::Status => store.catalog_status(env_num).await,
                            CatalogOperation::Start => store.catalog_start(env_num).await,
                            CatalogOperation::Check(entries) => {
                                store.catalog_check(env_num, &entries).await
                            }
                            CatalogOperation::Apply(entries) => {
                                store
                                    .catalog_apply(
                                        env_num,
                                        &environment.env_key,
                                        &entries,
                                        &state.text_store,
                                    )
                                    .await
                            }
                            CatalogOperation::Error(error) => {
                                store.catalog_error(env_num, &error).await
                            }
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
