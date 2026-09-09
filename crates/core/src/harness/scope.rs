//! Task-local managed settings for drivers which reload their workdir config.
use super::CodexSettings;
tokio::task_local! { static SETTINGS: Option<CodexSettings>; }

pub fn current() -> Option<CodexSettings> {
    SETTINGS.try_with(Clone::clone).ok().flatten()
}

pub fn with_settings<F: std::future::Future>(
    settings: Option<CodexSettings>,
    future: F,
) -> impl std::future::Future<Output = F::Output> {
    // Return the scope directly; another async wrapper duplicates a potentially
    // large orchestrator future while it is moved into the task-local scope.
    SETTINGS.scope(settings, future)
}
