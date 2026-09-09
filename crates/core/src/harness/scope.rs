//! Task-local settings for drivers which reload their workdir config.
use super::{CodexSettings, RuntimeSettings};
tokio::task_local! {
    static SETTINGS: Option<CodexSettings>;
    static RUNTIME: RuntimeSettings;
}

pub fn current() -> Option<CodexSettings> {
    SETTINGS.try_with(Clone::clone).ok().flatten()
}
pub fn current_runtime() -> Option<RuntimeSettings> {
    RUNTIME.try_with(Clone::clone).ok()
}

pub fn with_settings<F: std::future::Future>(
    settings: Option<CodexSettings>,
    future: F,
) -> impl std::future::Future<Output = F::Output> {
    with_execution(settings, current_runtime().unwrap_or_default(), future)
}

pub fn with_execution<F: std::future::Future>(
    settings: Option<CodexSettings>,
    runtime: RuntimeSettings,
    future: F,
) -> impl std::future::Future<Output = F::Output> {
    SETTINGS.scope(settings, RUNTIME.scope(runtime, future))
}
