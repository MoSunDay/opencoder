//! Native computer-use agent loop, distilled from cua's perceive->act cycle
//! (github.com/trycua/cua). cua drives a computer-use agent as a tight loop:
//! observe (screenshot) -> reason (LLM) -> act (mouse/key) -> repeat, until the
//! model reports completion. This module keeps that shape but makes the
//! environment pluggable via [`ComputerUseExecutor`], so the same loop can run
//! against an Anthropic/OpenAI computer-use tool, a local sandbox, or a test
//! double.
//!
//! The model-facing prompting and tool-call parsing live in the session
//! `computer_use` tool (`crates/session/src/tools/computer_use.rs`); this module
//! owns only the step budget + completion guard so it stays backend-agnostic and
//! unit-testable.

use anyhow::Result;

/// A single atomic action the model requested (cua's "act" step), serialized
/// 1:1 from the provider's computer-use tool call (`click`, `type`, `key`, ...).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ComputerAction {
    /// Provider-defined action kind (e.g. `screenshot`, `type`, `click`, `key`).
    #[serde(rename = "type")]
    pub kind: String,
    /// Remaining action fields forwarded verbatim (coordinate, text, key, ...).
    #[serde(flatten)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl ComputerAction {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            fields: serde_json::Map::new(),
        }
    }
    pub fn with(mut self, key: &str, value: serde_json::Value) -> Self {
        self.fields.insert(key.to_string(), value);
        self
    }
}

/// Outcome of executing one [`ComputerAction`]: the next observation fed back
/// into the model (typically a base64 screenshot) plus a textual annotation.
#[derive(Debug, Clone, Default)]
pub struct Observation {
    /// Base64-encoded screenshot / image the model reasons over next.
    pub screenshot_b64: Option<String>,
    /// Free-text status (e.g. "clicked (120, 340)", "task complete").
    pub text: String,
    /// `true` when the executor believes the overall task is finished and the
    /// loop should stop before issuing another action.
    pub done: bool,
}

/// Pluggable environment for [`ComputerUseLoop`]. A provider sandbox
/// (Anthropic/OpenAI computer-use) implements this against its real backend;
/// tests implement it against [`RecordingExecutor`].
#[async_trait::async_trait]
pub trait ComputerUseExecutor: Send + Sync {
    /// Seed observation (the initial screenshot) before any action runs.
    async fn initial_observation(&self) -> Result<Observation>;
    /// Apply one model action inside the sandbox and return what changed.
    async fn execute(&self, action: &ComputerAction) -> Result<Observation>;
}

/// Reason the loop stopped: the executor signalled completion or the step
/// budget ran out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOutcome {
    Done,
    MaxStepsReached,
}

/// Native perceive->act loop. Owns only the step count + completion guard so it
/// stays backend-agnostic and unit-testable.
pub struct ComputerUseLoop<'a> {
    executor: &'a dyn ComputerUseExecutor,
    max_steps: usize,
}

impl<'a> ComputerUseLoop<'a> {
    pub fn new(executor: &'a dyn ComputerUseExecutor, max_steps: usize) -> Self {
        Self { executor, max_steps }
    }

    /// Drive the loop with a closure that, given the latest observation,
    /// decides the next action (or `None` to stop). The live model path is
    /// driven from the session `computer_use` tool; this entry point covers the
    /// deterministic/test path and any pre-computed action plan.
    pub async fn run(
        self,
        mut next_action: impl FnMut(&Observation) -> Option<ComputerAction>,
    ) -> Result<(LoopOutcome, Observation)> {
        let mut obs = self.executor.initial_observation().await?;
        if obs.done {
            return Ok((LoopOutcome::Done, obs));
        }
        for _ in 0..self.max_steps {
            let Some(action) = next_action(&obs) else {
                return Ok((LoopOutcome::Done, obs));
            };
            obs = self.executor.execute(&action).await?;
            if obs.done {
                return Ok((LoopOutcome::Done, obs));
            }
        }
        Ok((LoopOutcome::MaxStepsReached, obs))
    }
}

/// Test / dry-run executor: records every action it receives and never produces
/// a real screenshot. Returns each action's kind as the observation text, and
/// marks itself `done` only when handed an action whose kind is `done`. Used by
/// the unit tests and by the first-version `computer_use` session tool.
pub struct RecordingExecutor {
    pub actions: std::sync::Mutex<Vec<ComputerAction>>,
}

impl Default for RecordingExecutor {
    fn default() -> Self {
        Self {
            actions: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl ComputerUseExecutor for RecordingExecutor {
    async fn initial_observation(&self) -> Result<Observation> {
        Ok(Observation {
            text: "initial screenshot".into(),
            ..Default::default()
        })
    }
    async fn execute(&self, action: &ComputerAction) -> Result<Observation> {
        self.actions.lock().unwrap().push(action.clone());
        let done = action.kind == "done";
        Ok(Observation {
            text: format!("executed {}", action.kind),
            done,
            ..Default::default()
        })
    }
}

/// Names the production provider backends the v1 `computer_use` tool targets.
/// Real execution against these providers' computer-use endpoints (Anthropic
/// `computer_20250124`, OpenAI `computer_use_preview`) is the next milestone;
/// the loop + executor abstraction here are ready to receive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBackend {
    Anthropic,
    OpenAi,
}

/// Configuration for the production provider-sandbox executor (declared here so
/// the capability surface is complete; the session tool fills in live execution
/// once a sandbox backend is connected).
#[derive(Debug, Clone)]
pub struct LlmProviderExecutor {
    pub backend: ProviderBackend,
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loop_stops_on_done_action() {
        let exec = RecordingExecutor::default();
        let plan = [
            ComputerAction::new("click").with("x", 10.into()),
            ComputerAction::new("done"),
        ];
        let mut i = 0;
        let (outcome, obs) = ComputerUseLoop::new(&exec, 10)
            .run(|_| {
                let a = plan.get(i).cloned();
                i += 1;
                a
            })
            .await
            .unwrap();
        assert_eq!(outcome, LoopOutcome::Done);
        assert!(obs.done);
        assert_eq!(exec.actions.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn loop_stops_when_closure_returns_none() {
        let exec = RecordingExecutor::default();
        let mut emitted = 0;
        let (outcome, _) = ComputerUseLoop::new(&exec, 10)
            .run(|_| {
                emitted += 1;
                if emitted <= 1 {
                    Some(ComputerAction::new("click"))
                } else {
                    None
                }
            })
            .await
            .unwrap();
        assert_eq!(outcome, LoopOutcome::Done);
        assert_eq!(exec.actions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn loop_respects_max_steps() {
        let exec = RecordingExecutor::default();
        let (outcome, _) = ComputerUseLoop::new(&exec, 3)
            .run(|_| Some(ComputerAction::new("click")))
            .await
            .unwrap();
        assert_eq!(outcome, LoopOutcome::MaxStepsReached);
        assert_eq!(exec.actions.lock().unwrap().len(), 3);
    }
}
