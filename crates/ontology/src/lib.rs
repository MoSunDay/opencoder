//! Ontology definitions, entities and relationships backed by a dedicated SQLite database.
mod api;
mod auth;
mod catalog;
mod database;
mod domain;
mod error;
mod freshness;
mod http;
mod mutations;
mod system_catalog;
mod text_store;

pub use catalog::CatalogOperation;
pub use database::{SchemaPolicy, SCHEMA_CONTRACT};
pub use domain::{FreshnessItemStatus, FreshnessRun, FreshnessRunItem, FreshnessRunStatus};
pub use error::AppError;
pub use freshness::FreshnessOperation;
pub use http::{router, AppState};
