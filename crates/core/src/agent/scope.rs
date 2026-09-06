//! Task-scoped resource roots keep concurrent executions on their own snapshot.
use std::{future::Future, path::PathBuf};
tokio::task_local! { static ROOT: Option<PathBuf>; }

pub fn current_root() -> Option<PathBuf> {
    ROOT.try_with(Clone::clone).ok().flatten()
}
pub async fn with_root<F: Future>(root: Option<PathBuf>, future: F) -> F::Output {
    ROOT.scope(root, future).await
}
pub fn with_root_sync<T>(root: Option<PathBuf>, f: impl FnOnce() -> T) -> T {
    ROOT.sync_scope(root, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_scopes_do_not_change_the_global_root() {
        let _guard = super::super::meta::tests::OVERRIDE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let original = super::super::agents_dir();
                let (a, b) = tokio::join!(
                    with_root(Some("/a".into()), async {
                        tokio::task::yield_now().await;
                        super::super::agents_dir()
                    }),
                    with_root(Some("/b".into()), async { super::super::agents_dir() })
                );
                assert_eq!(a, Some("/a".into()));
                assert_eq!(b, Some("/b".into()));
                assert_eq!(super::super::agents_dir(), original);
            });
    }
}
