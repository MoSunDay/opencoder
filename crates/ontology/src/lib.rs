//! Ontology definitions, entities and relationships backed by a dedicated SQLite database.
mod api;
mod auth;
mod database;
mod domain;
mod error;
mod http;
mod mutations;
mod system_catalog;
mod text_store;

pub use error::AppError;
pub use http::{router, AppState};
