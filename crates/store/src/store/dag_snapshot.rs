//! Compact run state: lifecycle events only, never step log payloads.

#[derive(Debug, Clone, Default)]
pub struct DagStepSnapshot {
    pub head_seq: i64,
    pub steps: Vec<DagStepEvent>,
}

#[derive(Debug, Clone)]
pub struct DagStepEvent {
    pub name: String,
    pub seq: i64,
    pub started: bool,
    pub at_ms: i64,
    pub ok: bool,
    pub error: Option<String>,
}
