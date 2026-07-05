use std::path::Path;

use anyhow::{Context, Result};
use async_trait::async_trait;
use libsql::{Builder, Connection};
use tracing::debug;

use crate::store::Store;
use crate::types::{Delivery, ImportReport, SessionEventRecord, SessionFilter, SessionInput, SessionListItem, SessionMeta, SessionPatch};

mod events;
mod inputs;
mod messages;
mod schema;
mod sessions;

/// Primary `Store` implementation backed by libsql (embedded local SQLite, WAL).
///
/// Holds ONE connection obtained at open time; each operation clones it. libsql
/// connection clones share the same underlying database — this makes in-memory
/// databases work correctly across operations (a fresh `db.connect()` per op
/// would hand back an empty `:memory:` db every time) while file-backed dbs
/// still get WAL semantics. All SQL lives in free functions in the submodules
/// so the backend can be swapped without touching callers.
pub struct LibsqlStore {
    conn: Connection,
}

impl LibsqlStore {
    /// Open (or create) a libsql database file and bootstrap the schema.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Builder::new_local(path.as_ref())
            .build()
            .await
            .with_context(|| format!("open libsql db at {}", path.as_ref().display()))?;
        let conn = db.connect().context("connect libsql")?;
        schema::apply_connection_pragmas(&conn).await?;
        schema::bootstrap(&conn).await?;
        let store = LibsqlStore { conn };
        debug!(backend = "libsql", "store opened");
        Ok(store)
    }

    /// Open an in-memory database (used by tests and ephemeral runs).
    pub async fn open_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:").build().await.context("open in-memory db")?;
        let conn = db.connect().context("connect in-memory")?;
        schema::apply_connection_pragmas(&conn).await?;
        schema::bootstrap(&conn).await?;
        Ok(LibsqlStore { conn })
    }

    /// Acquire a connection that shares the underlying database. Cheap clone.
    pub async fn conn(&self) -> Result<Connection> {
        Ok(self.conn.clone())
    }
}

#[async_trait]
impl Store for LibsqlStore {
    fn backend_name(&self) -> &'static str {
        "libsql"
    }

    async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        let conn = self.conn().await?;
        sessions::create(&conn, meta).await
    }
    async fn get_session(&self, id: &str) -> Result<Option<SessionMeta>> {
        let conn = self.conn().await?;
        sessions::get(&conn, id).await
    }
    async fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionListItem>> {
        let conn = self.conn().await?;
        sessions::list(&conn, filter).await
    }
    async fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<()> {
        let conn = self.conn().await?;
        sessions::update(&conn, id, patch).await
    }
    async fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.conn().await?;
        sessions::delete(&conn, id).await
    }

    async fn append_message(&self, session_id: &str, msg: &opencode_core::Message) -> Result<i64> {
        let conn = self.conn().await?;
        messages::append(&conn, session_id, msg).await
    }
    async fn append_messages(&self, session_id: &str, msgs: &[opencode_core::Message]) -> Result<Vec<i64>> {
        let conn = self.conn().await?;
        messages::append_many(&conn, session_id, msgs).await
    }
    async fn load_messages(&self, session_id: &str) -> Result<Vec<opencode_core::Message>> {
        let conn = self.conn().await?;
        messages::load(&conn, session_id).await
    }
    async fn last_message_seq(&self, session_id: &str) -> Result<i64> {
        let conn = self.conn().await?;
        messages::last_seq(&conn, session_id).await
    }

    async fn admit_input(&self, input: &SessionInput) -> Result<i64> {
        let conn = self.conn().await?;
        inputs::admit(&conn, input).await
    }
    async fn pending_inputs(&self, session_id: &str, delivery: Delivery) -> Result<Vec<SessionInput>> {
        let conn = self.conn().await?;
        inputs::pending(&conn, session_id, delivery).await
    }
    async fn promote_inputs(&self, session_id: &str, up_to_admitted_seq: i64, delivery: Delivery) -> Result<Vec<i64>> {
        let conn = self.conn().await?;
        inputs::promote(&conn, session_id, up_to_admitted_seq, delivery).await
    }
    async fn promote_next_queued(&self, session_id: &str) -> Result<Option<i64>> {
        let conn = self.conn().await?;
        inputs::promote_next_queued(&conn, session_id).await
    }
    async fn claim_next_queue(&self, session_id: &str) -> Result<Option<SessionInput>> {
        let conn = self.conn().await?;
        inputs::claim_next_queue(&conn, session_id).await
    }
    async fn delete_input(&self, input_id: i64) -> Result<()> {
        let conn = self.conn().await?;
        inputs::delete_input(&conn, input_id).await
    }

    async fn append_event(&self, event: &SessionEventRecord) -> Result<i64> {
        let conn = self.conn().await?;
        events::append(&conn, event).await
    }
    async fn events_after(&self, session_id: &str, after_seq: i64) -> Result<Vec<SessionEventRecord>> {
        let conn = self.conn().await?;
        events::after(&conn, session_id, after_seq).await
    }

    async fn import_messages(&self, session_id: &str, msgs: &[opencode_core::Message]) -> Result<ImportReport> {
        let conn = self.conn().await?;
        messages::import(&conn, session_id, msgs).await
    }
}
