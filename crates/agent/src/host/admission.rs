//! Bound cold creation per runtime, including retained legacy runtimes.
use opencoder_core::fleet::{NodeOperation, RpcReply};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

const MAX_CREATING_PER_RUNTIME: usize = 4;

#[derive(Default)]
pub(super) struct Creations(Mutex<HashMap<String, HashSet<String>>>);

pub(super) struct Creation<'a> {
    owner: &'a Creations,
    runtime: String,
    execution: String,
}

impl Creations {
    pub(super) fn begin(
        &self,
        runtime: &str,
        operation: &NodeOperation,
    ) -> Result<Option<Creation<'_>>, RpcReply> {
        let NodeOperation::Create { assignment } = operation else {
            return Ok(None);
        };
        let id = &assignment.index.id;
        let mut pending = self.0.lock().unwrap();
        let executions = pending.entry(runtime.into()).or_default();
        if executions.contains(id) || executions.len() >= MAX_CREATING_PER_RUNTIME {
            return Err(RpcReply::error(
                503,
                "runtime creation is busy; retry using the same execution id",
            ));
        }
        executions.insert(id.clone());
        Ok(Some(Creation {
            owner: self,
            runtime: runtime.into(),
            execution: id.clone(),
        }))
    }
}

impl Drop for Creation<'_> {
    fn drop(&mut self) {
        let mut pending = self.owner.0.lock().unwrap();
        if let Some(executions) = pending.get_mut(&self.runtime) {
            executions.remove(&self.execution);
            if executions.is_empty() {
                pending.remove(&self.runtime);
            }
        }
    }
}

/// Match the control-plane budgets. A disconnected/timed-out caller must not
/// leave an unbounded Host HTTP request occupying a fleet channel permit.
pub(super) fn request_timeout(operation: &NodeOperation) -> Duration {
    Duration::from_secs(if matches!(operation, NodeOperation::Create { .. }) {
        60
    } else {
        15
    })
}
