use std::path::Path;

use anyhow::Result;
use turso::Connection;

#[path = "execution/brain.rs"]
mod brain;
#[path = "execution/brain_layered.rs"]
mod brain_layered;
#[path = "conversation/chat_tables.rs"]
mod chat_tables;
#[path = "bootstrap/connection/mod.rs"]
mod connection;
#[path = "execution/dag.rs"]
mod dag;
#[path = "execution/dag_events.rs"]
mod dag_events;
#[path = "conversation/events.rs"]
mod events;
#[path = "impl_store.rs"]
mod impl_store;
#[path = "conversation/inputs.rs"]
mod inputs;
#[path = "conversation/messages.rs"]
mod messages;
#[path = "execution/node_state.rs"]
mod node_state;
#[path = "execution/node_tasks.rs"]
mod node_tasks;
#[path = "execution/nodes.rs"]
mod nodes;
#[path = "projects/project.rs"]
mod project;
#[path = "projects/project_links.rs"]
mod project_links;
#[path = "projects/project_runs.rs"]
mod project_runs;
#[path = "execution/schedule.rs"]
mod schedule;
#[path = "bootstrap/schema.rs"]
pub(crate) mod schema;
#[path = "conversation/sessions.rs"]
mod sessions;
#[path = "conversation/subagent_tasks.rs"]
mod subagent_tasks;
#[path = "execution/team_runs.rs"]
mod team_runs;
#[path = "projects/todos.rs"]
mod todos;
#[path = "tx.rs"]
mod tx;
#[path = "identity/users.rs"]
mod users;

/// Database schema watermark; v4 brain tables are additive and must not move it.
pub fn schema_watermark() -> i64 {
    schema::SCHEMA_VERSION
}

/// Embedded Turso store with independent read connections and serialized writes.
/// Complete operations execute on dedicated database workers.
#[derive(Clone)]
pub struct TursoStore {
    db: opencoder_database::Database,
}

impl TursoStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            db: connection::open_file(path.as_ref()).await?,
        })
    }

    pub async fn open_memory() -> Result<Self> {
        Self::open(":memory:").await
    }

    /// Independent diagnostic connection; normal operations use the bounded pool.
    pub async fn conn(&self) -> Result<Connection> {
        self.db.connect().await
    }
}
