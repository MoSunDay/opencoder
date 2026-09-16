//! Compatibility types for historical decision-tree and playbook queries.
//! All writes and execution require the version 2 graph contract.
use crate::{
    plan::{DecisionTree, DispatchOutcome},
    playbook::PlaybookSpec,
    Runtime,
};
use anyhow::{bail, Result};
use opencoder_store::{BrainPlanRecord, BrainPlaybookRecord};
pub const PLANNER_FRAMEWORK_PROMPT: &str = crate::activation::PROMPT;
pub const PLAYBOOK_FRAMEWORK_PROMPT: &str = crate::activation::PROMPT;
#[derive(Debug, Clone)]
pub struct Dispatched {
    pub record: BrainPlanRecord,
    pub outcome: DispatchOutcome,
    pub planned_fresh: bool,
}
#[derive(Debug, Clone)]
pub struct PlannedPlaybook {
    pub record: BrainPlaybookRecord,
    pub spec: PlaybookSpec,
    pub planned_fresh: bool,
}
pub fn situation_digest(situation: &str) -> String {
    crate::execution::fingerprint(&situation.trim())
}
impl Runtime {
    pub async fn plan_decision_tree(
        &self,
        _: &str,
        _: &str,
        _: u32,
        _: i64,
    ) -> Result<(BrainPlanRecord, DecisionTree)> {
        bail!(crate::graph::MIGRATION)
    }
    pub async fn dispatch_decision_tree(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(BrainPlanRecord, DispatchOutcome)> {
        bail!(crate::graph::MIGRATION)
    }
    pub async fn dispatch_or_plan(
        &self,
        _: &str,
        _: &str,
        _: u32,
        _: bool,
        _: i64,
    ) -> Result<Dispatched> {
        bail!(crate::graph::MIGRATION)
    }
    pub async fn plan_playbook(
        &self,
        _: &str,
        _: &str,
        _: u32,
        _: bool,
        _: i64,
    ) -> Result<PlannedPlaybook> {
        bail!(crate::graph::MIGRATION)
    }
}
