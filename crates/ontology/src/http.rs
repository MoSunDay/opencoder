use crate::{api, auth::Actor, database::Database, error::AppError, text_store::TextStore};
use axum::{middleware, Extension, Json, Router};
use std::path::Path;

#[derive(Clone)]
pub struct AppState {
    pub(crate) database: Database,
    pub(crate) text_store: TextStore,
}

impl AppState {
    pub async fn open(database: &Path, files: &Path) -> Result<Self, AppError> {
        let text_store = TextStore::initialize(files.to_path_buf()).await?;
        let database = Database::open(database, &files.canonicalize()?).await?;
        Ok(Self {
            database,
            text_store,
        })
    }

    pub async fn drained(&self) {
        self.database.drained().await;
    }

    pub async fn ready(&self) -> Result<(), AppError> {
        self.database.ping().await?;
        self.text_store.ping().await
    }
}

/// Mounted under `/api/ontology` by the authenticated control-plane router.
pub fn router(state: AppState) -> Router {
    api::routes()
        .layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::mutations::transaction,
        ))
        .layer(middleware::from_fn(crate::auth::identify))
        .with_state(state)
}

pub(crate) async fn session(Extension(actor): Extension<Actor>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "principal":{"external_id":actor.external_id,"display_name":actor.display_name,
            "kind":if actor.is_service {"service"} else {"user"}},
        "capabilities":if actor.is_admin {vec!["view_business_understanding","manage_business_understanding"]}
            else {vec!["view_business_understanding"]}
    }))
}
