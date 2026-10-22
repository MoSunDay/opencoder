use super::*;

impl Journal {
    /// Install metadata only after the original record is durably available.
    pub(super) fn install(&mut self, record: Record, location: RecordLocation) {
        let id = record.assignment.index.id.clone();
        self.revision = self
            .revision
            .checked_add(1)
            .expect("journal revision exhausted");
        self.summaries.insert(
            id.clone(),
            summary::Summary::new(&record, location, self.revision),
        );
        self.cache.lock().unwrap().remove(&id);
        self.inspection_cache.lock().unwrap().remove(&id);
        if self.report_candidates.contains(&id) {
            self.records.insert(id, report_projection(&record));
        } else {
            self.records.remove(&id);
        }
    }
}

/// The report path must never keep frozen configuration, definitions or events.
/// This private Record-shaped view is consumed only by existing outbox helpers.
fn report_projection(record: &Record) -> Record {
    let input = &record.assignment.request.input;
    let mut projected_input = serde_json::Map::new();
    for key in ["schema_version", "brain_layered"] {
        if let Some(value) = input.get(key) {
            projected_input.insert(key.into(), value.clone());
        }
    }
    if let Some(value) = input.get("layered_request") {
        projected_input.insert(
            "layered_request".into(),
            if value.is_object() {
                match value.get("parent") {
                    Some(parent) => json!({"parent":parent}),
                    None => json!({}),
                }
            } else {
                value.clone()
            },
        );
    }
    let mut annotations = serde_json::Map::new();
    for key in [
        "layered_wake_ack",
        "layered_guidance_ack",
        "brain_layered_ack",
        "layered_dispatch_acks",
        "layered_cancel_acks",
    ] {
        if let Some(value) = record.annotations.get(key) {
            annotations.insert(key.into(), value.clone());
        }
    }
    Record {
        annotations: Value::Object(annotations),
        queue: None,
        assignment: Assignment {
            index: record.assignment.index.clone(),
            request: CreateExecution {
                id: record.assignment.request.id.clone(),
                kind: record.assignment.request.kind,
                input: Value::Object(projected_input),
                target: None,
                node_id: None,
            },
            definition: None,
            runtime: None,
            codex: None,
            private_context: None,
        },
        result: json!({"phase":record.result.get("phase")}),
        error: None,
        events: record
            .events
            .last()
            .map(|event| SseEvt {
                seq: event.seq,
                ts: 0,
                kind: String::new(),
                data: Value::Null,
            })
            .into_iter()
            .collect(),
        lifecycle: Lifecycle::default(),
    }
}
