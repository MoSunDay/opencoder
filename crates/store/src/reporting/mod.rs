//! Durable report headers and monotonic acknowledgement cursors.
#[derive(Clone, Debug)]
pub struct ReportAck {
    pub execution_id: String,
    pub action: String,
    pub item: String,
    pub cursor: u64,
}
#[derive(Clone, Debug)]
pub struct PendingReport {
    pub key: String,
    pub execution_id: String,
    pub action: String,
    pub item: String,
    pub cursor: u64,
    pub payload: serde_json::Value,
}
#[derive(Clone, Debug)]
pub struct ReportRunState {
    pub generation: u64,
    pub last_event_seq: u64,
}
