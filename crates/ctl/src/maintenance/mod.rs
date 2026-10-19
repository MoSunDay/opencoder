//! Local engine-aware maintenance; these commands never resolve HTTP credentials.
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub enum StorageAction {
    /// Serve SQL requests on stdin/stdout using one native connection.
    Serve {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        read_only: bool,
    },
    /// Execute a read-only SQL query and return columns and rows as JSON.
    Query {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        sql: String,
        /// SQL arguments as a JSON array of scalar values.
        #[arg(long, default_value = "[]")]
        params: String,
    },
    /// Copy one consistent snapshot to a new file, without stopping its writers.
    Backup {
        #[arg(long)]
        database: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Flush WAL after every database writer has been stopped.
    Checkpoint {
        #[arg(long)]
        database: PathBuf,
        #[arg(long, required = true)]
        writers_stopped: bool,
    },
}

pub async fn run(action: &StorageAction) -> Result<i32> {
    use opencoder_database::maintenance;
    let result = match action {
        StorageAction::Serve {
            database,
            read_only,
        } => {
            maintenance::serve(database.clone(), *read_only).await?;
            return Ok(0);
        }
        StorageAction::Query {
            database,
            sql,
            params,
        } => {
            maintenance::query(database.clone(), sql.clone(), serde_json::from_str(params)?).await?
        }
        StorageAction::Backup { database, output } => {
            maintenance::backup(database.clone(), output.clone()).await?;
            serde_json::json!({"ok": true, "engine": "turso", "output": output})
        }
        StorageAction::Checkpoint {
            database,
            writers_stopped,
        } => {
            anyhow::ensure!(*writers_stopped, "checkpoint requires stopped writers");
            maintenance::checkpoint(database.clone()).await?;
            serde_json::json!({"ok": true, "engine": "turso"})
        }
    };
    println!("{result}");
    Ok(0)
}
