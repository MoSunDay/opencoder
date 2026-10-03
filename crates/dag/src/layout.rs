use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

pub fn run_parent(data: &Path, dag: &str, created_at: i64) -> Result<PathBuf> {
    ensure!(data.is_absolute(), "DAG data directory must be absolute");
    ensure!(
        crate::artifacts::validate_dag_id(dag),
        "invalid DAG identifier"
    );
    let date = DateTime::<Utc>::from_timestamp_millis(created_at)
        .context("invalid DAG creation date")?
        .format("%Y-%m-%d")
        .to_string();
    Ok(data.join(date).join(dag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_uses_fixed_utc_creation_date_and_safe_identifiers() {
        assert_eq!(
            run_parent(Path::new("/data/dags"), "build", 0).unwrap(),
            Path::new("/data/dags/1970-01-01/build")
        );
        assert!(run_parent(Path::new("/data/dags"), "../escape", 0).is_err());
        assert_eq!(
            run_parent(Path::new("/data/dags"), "示例工作流", 0).unwrap(),
            Path::new("/data/dags/1970-01-01/示例工作流")
        );
        assert!(run_parent(Path::new("relative"), "build", 0).is_err());
        assert!(run_parent(Path::new("/data/dags"), "build", i64::MAX).is_err());
    }
}
