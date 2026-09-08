//! Node wiring for the external viking strong/weak dependency analysis.
//!
//! Scenario 1 of the external-agent integration: a dedicated
//! `dependency-analysis` persona session gets a `dependency_analysis` tool
//! that submits a five-tuple to the hosted VTA trigger endpoint (through the
//! reviewed `viking-cli`), polls the analysis workflow to a terminal state,
//! and appends the validated verdict to the persona's prompt-pool `how.md`
//! as a new version.

mod append;
mod cli;
mod flow;
mod model;
mod tool;

use crate::Worker;
use std::sync::Arc;

/// The custom-agent persona this integration is registered to.
pub(crate) const PERSONA: &str = "dependency-analysis";

/// The tool is attached only to sessions targeting the dedicated persona —
/// never to ordinary agent sessions.
pub(crate) fn should_install(target: &str) -> bool {
    target == PERSONA
}

/// Register the `dependency_analysis` tool for one persona session and hold
/// the registration for the session's lifetime (mirrors
/// `maintenance_tools::install`). A missing token/CLI is warn-only: the
/// session still starts and the tool error surfaces if invoked.
pub(crate) fn install(worker: &Worker, id: &str) {
    let tool = match tool::DependencyTool::from_env() {
        Ok(tool) => tool,
        Err(error) => {
            tracing::warn!(%error, "dependency_analysis tool unavailable for this session");
            return;
        }
    };
    worker
        .inner
        .dependency
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id.into())
        .or_insert_with(|| opencoder_session::extensions::register(id, vec![Arc::new(tool)]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_persona_gated() {
        assert!(should_install(PERSONA));
        assert!(!should_install("act"));
        assert!(!should_install("dependency-analysis-other"));
    }
}
