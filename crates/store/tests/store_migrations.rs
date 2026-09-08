//! Schema migration integration suites, grouped by historical feature.

#[path = "store_migrations/catalog.rs"]
mod catalog;
#[path = "store_migrations/early.rs"]
mod early;
#[path = "store_migrations/middle.rs"]
mod middle;
#[path = "store_migrations/project_replay.rs"]
mod project_replay;
#[path = "store_migrations/sessions.rs"]
mod sessions;
