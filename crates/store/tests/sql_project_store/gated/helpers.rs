use super::*;

pub(super) fn env_dsn(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
pub(super) async fn eventually<F, Fut>(what: &str, mut probe: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..100 {
        if probe().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}
pub(super) fn storage(backend: StorageBackend, dsn: &str) -> StorageConfig {
    StorageConfig {
        backend,
        mysql: (backend == StorageBackend::Mysql).then_some(dsn.to_string()),
        starrocks: (backend == StorageBackend::Starrocks).then_some(dsn.to_string()),
    }
}
