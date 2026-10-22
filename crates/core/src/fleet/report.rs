use super::ExecutionIndex;
use serde::{Deserialize, Serialize};

pub const INDEX_REPORT_DELTA_HEADER: &str = "x-opencoder-index-report-delta";
/// Version 2 keeps each connection's baseline while Servers share storage.
/// Version 1 peers use acknowledged full reports during a Host-first upgrade.
pub const INDEX_REPORT_DELTA_VERSION: &str = "2";

pub const INDEX_REPORT_BATCH_SIZE: usize = 256;
pub const INDEX_REPORT_ACK_HEADER: &str = "x-opencoder-index-report-ack";
pub const INDEX_REPORT_ACK_ACTION: &str = "transport.index_report_ack_v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexReportMode {
    #[default]
    Full,
    Delta,
}
impl IndexReportMode {
    fn is_full(&self) -> bool {
        *self == Self::Full
    }
}

/// Ordered fragments; delta is legal only after explicit handshake negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReportEnvelope {
    pub report_id: u64,
    /// Begin metadata. The base is the last committed load snapshot sequence.
    #[serde(default, skip_serializing_if = "IndexReportMode::is_full")]
    pub mode: IndexReportMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sequence: Option<u64>,
    /// Batch removals affect inventory membership, never execution history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub acknowledge: bool,
    #[serde(flatten)]
    pub part: IndexReportPart,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum IndexReportPart {
    Begin,
    Batch { records: Vec<ExecutionIndex> },
    End,
}

impl IndexReportEnvelope {
    pub fn with_ack(mut self, enabled: bool) -> Self {
        self.acknowledge = enabled;
        self
    }

    pub fn begin(report_id: u64) -> Self {
        Self {
            report_id,
            mode: IndexReportMode::Full,
            base_sequence: None,
            removed_ids: Vec::new(),
            acknowledge: false,
            part: IndexReportPart::Begin,
        }
    }

    pub fn batch(report_id: u64, records: Vec<ExecutionIndex>) -> Self {
        Self {
            report_id,
            mode: IndexReportMode::Full,
            base_sequence: None,
            removed_ids: Vec::new(),
            acknowledge: false,
            part: IndexReportPart::Batch { records },
        }
    }

    pub fn end(report_id: u64) -> Self {
        Self {
            report_id,
            mode: IndexReportMode::Full,
            base_sequence: None,
            removed_ids: Vec::new(),
            acknowledge: false,
            part: IndexReportPart::End,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_report_wire_format_stays_identical_and_new_flow_is_explicit() {
        assert_eq!(
            serde_json::to_value(IndexReportEnvelope::begin(1)).unwrap(),
            serde_json::json!({"report_id":1,"phase":"begin"})
        );
        let acknowledged = IndexReportEnvelope::begin(1).with_ack(true);
        let value = serde_json::to_value(acknowledged).unwrap();
        assert_eq!(value["acknowledge"], true);
        let legacy: IndexReportEnvelope =
            serde_json::from_value(serde_json::json!({"report_id":2,"phase":"end"})).unwrap();
        assert!(!legacy.acknowledge);
    }
    #[test]
    fn delta_metadata_and_removals_round_trip_without_changing_legacy_batches() {
        let mut begin = IndexReportEnvelope::begin(2).with_ack(true);
        begin.mode = IndexReportMode::Delta;
        begin.base_sequence = Some(9007199254740993);
        let wire = serde_json::to_value(&begin).unwrap();
        assert_eq!(wire["phase"], "begin");
        assert_eq!(wire["mode"], "delta");
        let decoded: IndexReportEnvelope = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.base_sequence, begin.base_sequence);
        let mut batch = IndexReportEnvelope::batch(2, vec![]).with_ack(true);
        batch.removed_ids = vec!["agent-removed".into()];
        let decoded: IndexReportEnvelope =
            serde_json::from_str(&serde_json::to_string(&batch).unwrap()).unwrap();
        assert_eq!(decoded.removed_ids, batch.removed_ids);
        assert_eq!(
            serde_json::to_value(IndexReportEnvelope::batch(1, vec![])).unwrap(),
            serde_json::json!({"report_id":1,"phase":"batch","records":[]})
        );
    }
}
