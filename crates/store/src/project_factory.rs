//! Project-data backend selection. The project module (goals / milestones /
//! todos / runs) is the only subsystem allowed to live outside the embedded
//! libsql store, so picking its backend is a dedicated factory instead of the
//! generic store opening path.

use std::sync::Arc;

use anyhow::Result;
use opencoder_core::{StorageBackend, StorageConfig};

use crate::{LibsqlStore, ProjectStore};

/// Pick the project-data backend. libsql shares the SAME store instance
/// (one connection, one db_lock); the optional mysql/starrocks backends are
/// feature-gated: with the feature compiled in the sql_store backend serves
/// the four project tables, without it the request refuses cleanly instead
/// of silently falling back to libsql.
pub async fn open_project_store(
    storage: &StorageConfig,
    libsql: Arc<LibsqlStore>,
) -> Result<Arc<dyn ProjectStore>> {
    match storage.backend {
        StorageBackend::Libsql => Ok(libsql),
        #[cfg(feature = "mysql")]
        StorageBackend::Mysql => crate::sql_store::open(storage).await,
        #[cfg(not(feature = "mysql"))]
        StorageBackend::Mysql => anyhow::bail!(
            "project storage backend 'mysql' requires building opencoder-store \
             with the mysql cargo feature (--features mysql)"
        ),
        #[cfg(feature = "starrocks")]
        StorageBackend::Starrocks => crate::sql_store::open(storage).await,
        #[cfg(not(feature = "starrocks"))]
        StorageBackend::Starrocks => anyhow::bail!(
            "project storage backend 'starrocks' requires building opencoder-store \
             with the starrocks cargo feature (--features starrocks)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn libsql_backend_shares_the_libsql_instance() {
        let libsql = Arc::new(LibsqlStore::open_memory().await.unwrap());
        let storage = StorageConfig::default();
        assert_eq!(storage.backend.as_str(), "libsql", "default backend");
        let projects = open_project_store(&storage, libsql).await.unwrap();
        assert_eq!(projects.project_backend_name(), "libsql");
    }

    #[tokio::test]
    async fn optional_backends_refuse_cleanly_without_their_feature() {
        let libsql = Arc::new(LibsqlStore::open_memory().await.unwrap());
        for name in ["mysql", "starrocks"] {
            let storage = StorageConfig {
                backend: StorageBackend::parse(name).unwrap(),
                ..Default::default()
            };
            let err = match open_project_store(&storage, libsql.clone()).await {
                Err(e) => e.to_string(),
                Ok(_) => panic!("{name} must refuse without a DSN"),
            };
            assert!(err.contains(name), "message names the backend: {err}");
            let feature_on = match name {
                "starrocks" => cfg!(feature = "starrocks"),
                _ => cfg!(feature = "mysql"),
            };
            if feature_on {
                // Feature compiled in but no DSN configured: refuse with the
                // DSN-missing error, still without touching libsql.
                assert!(err.contains("requires a DSN"), "{err}");
            } else {
                assert!(
                    err.contains("cargo feature"),
                    "message points at the feature gate: {err}"
                );
            }
        }
    }
}
