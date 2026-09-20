use opencoder_dag::DagEventIn;
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

/// Incremental, run-scoped step log producer. The run uploader owns delivery;
/// this component only frames output and never blocks a step.
#[derive(Clone)]
pub struct StepLog {
    step: String,
    instance: Option<usize>,
    tx: UnboundedSender<DagEventIn>,
}

impl StepLog {
    pub(crate) fn new(step: String, tx: UnboundedSender<DagEventIn>) -> Self {
        Self {
            step,
            tx,
            instance: None,
        }
    }

    pub(crate) fn with_instance(mut self, instance: Option<usize>) -> Self {
        self.instance = instance;
        self
    }

    pub(crate) fn push(&self, event: &str, data: &str) {
        let _ = self.tx.send(DagEventIn {
            kind: "step_log".into(),
            step: Some(self.step.clone()),
            payload: json!({"event": event, "data": data, "index": self.instance}),
            at_ms: opencoder_core::message::now_ms(),
        });
    }

    pub(crate) fn output(&self, stream: &str, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes).into_owned();
        self.push(stream, &text);
    }

    pub(crate) fn text_delta(&self, text: &str) {
        let _ = self.tx.send(DagEventIn {
            kind: "step_log".into(),
            step: Some(self.step.clone()),
            payload: json!({"event": "text_delta", "data": {"text": text}, "index": self.instance}),
            at_ms: opencoder_core::message::now_ms(),
        });
    }
}
