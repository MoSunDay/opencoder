//! Libsql connection bootstrap and its cold-start observability.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use libsql::{Builder, Connection};

use super::schema;

fn should_checkpoint_wal(path_existed: bool) -> bool {
    path_existed
}

pub(super) async fn open_file(path: &Path) -> Result<Connection> {
    let existed = path.exists();
    let total = std::time::Instant::now();

    let started = std::time::Instant::now();
    let db = Builder::new_local(path)
        .build()
        .await
        .with_context(|| format!("open libsql db at {}", path.display()))?;
    let conn = db.connect().context("connect libsql")?;
    let build_ms = started.elapsed().as_millis() as u64;

    let started = std::time::Instant::now();
    schema::apply_connection_pragmas(&conn).await?;
    let _ = conn.busy_timeout(Duration::from_secs(30));
    let pragma_ms = started.elapsed().as_millis() as u64;

    let started = std::time::Instant::now();
    schema::bootstrap(&conn).await?;
    let bootstrap_ms = started.elapsed().as_millis() as u64;

    let started = std::time::Instant::now();
    if should_checkpoint_wal(existed) {
        let _ = schema::checkpoint_wal(&conn).await;
    }
    let checkpoint_ms = started.elapsed().as_millis() as u64;
    log_open(
        path,
        total.elapsed().as_millis() as u64,
        [
            ("build", build_ms),
            ("pragmas", pragma_ms),
            ("bootstrap", bootstrap_ms),
            ("checkpoint", checkpoint_ms),
        ],
    );
    Ok(conn)
}

pub(super) async fn open_memory() -> Result<Connection> {
    let db = Builder::new_local(":memory:")
        .build()
        .await
        .context("open in-memory db")?;
    let conn = db.connect().context("connect in-memory")?;
    schema::apply_connection_pragmas(&conn).await?;
    let _ = conn.busy_timeout(Duration::from_secs(30));
    schema::bootstrap(&conn).await?;
    Ok(conn)
}

fn log_open(path: &Path, total_ms: u64, stages: [(&'static str, u64); 4]) {
    let [(_, build_ms), (_, pragma_ms), (_, bootstrap_ms), (_, checkpoint_ms)] = stages;
    tracing::info!(
        backend = "libsql", path = %path.display(), build_ms, pragma_ms,
        bootstrap_ms, checkpoint_ms, total_ms, "store opened"
    );
    if total_ms > 1000 || stages.iter().any(|&(_, ms)| ms > 1000) {
        let (slowest_stage, slowest_ms) = stages
            .into_iter()
            .max_by_key(|&(_, ms)| ms)
            .unwrap_or(("total", total_ms));
        tracing::warn!(
            backend = "libsql", path = %path.display(), slowest_stage,
            slowest_ms, total_ms, "slow store open"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::should_checkpoint_wal;

    #[test]
    fn checkpoint_gate_skips_fresh_file_and_runs_on_existing() {
        assert!(!should_checkpoint_wal(false));
        assert!(should_checkpoint_wal(true));
    }
}
