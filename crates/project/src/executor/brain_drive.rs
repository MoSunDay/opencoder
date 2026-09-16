//! Legacy Project brain executor is read-only; submit v2 brain runs instead.
use crate::{
    context::ProjectContext,
    executor::ResolvedExecutor,
    service::{Deps, ExecutorOverride},
};
use anyhow::Result;
use opencoder_store::ProjectTodoRecord;
use std::sync::Arc;
#[derive(Debug, Clone, Default)]
pub struct BrainTrace {
    pub capability_id: Option<String>,
    pub plan_id: Option<String>,
}
pub async fn resolve_brain(
    _: &Arc<Deps>,
    _: &ProjectTodoRecord,
    _: &ProjectContext,
    _: Option<&ExecutorOverride>,
) -> Result<(ResolvedExecutor, BrainTrace)> {
    anyhow::bail!(opencoder_brain::graph::MIGRATION)
}
