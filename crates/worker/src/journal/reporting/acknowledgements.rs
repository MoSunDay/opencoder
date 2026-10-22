use super::*;
use opencoder_store::reporting::ReportAck;

fn key(action: &str) -> Option<&'static str> {
    Some(match action {
        "layered_wake" => "layered_wake_ack",
        "layered_guidance" => "layered_guidance_ack",
        "layered_terminal" => parent::ACK,
        "layered_dispatch" => "layered_dispatch_acks",
        "layered_cancel" => "layered_cancel_acks",
        _ => return None,
    })
}
impl Journal {
    pub(crate) fn legacy_report_acks(&self) -> Vec<ReportAck> {
        let mut acks = Vec::new();
        for id in &self.report_candidates {
            let record = &self.records[id];
            for action in [
                "layered_wake",
                "layered_guidance",
                "layered_terminal",
                "layered_dispatch",
                "layered_cancel",
            ] {
                let value = &record.annotations[key(action).expect("ack key")];
                if let Some(cursor) = value.as_u64() {
                    acks.push(ReportAck {
                        execution_id: id.clone(),
                        action: action.into(),
                        item: String::new(),
                        cursor,
                    });
                } else if let Some(items) = value.as_object() {
                    for (item, confirmed) in items {
                        if confirmed == true {
                            acks.push(ReportAck {
                                execution_id: id.clone(),
                                action: action.into(),
                                item: item.clone(),
                                cursor: 1,
                            });
                        }
                    }
                }
            }
        }
        acks
    }
    pub(crate) fn apply_report_ack(&mut self, ack: &ReportAck, persist: bool) -> Result<()> {
        let Some(key) = key(&ack.action) else {
            return Ok(());
        };
        let Some(record) = self.records.get(&ack.execution_id) else {
            return Ok(());
        };
        let current = if ack.item.is_empty() {
            record.annotations[key].as_u64().unwrap_or(0)
        } else {
            u64::from(record.annotations[key][&ack.item] == true)
        };
        let already_present = if ack.item.is_empty() {
            record.annotations[key].is_number()
        } else {
            record.annotations[key][&ack.item] == true
        };
        if already_present && current >= ack.cursor {
            return Ok(());
        }
        let apply = |record: &mut Record| {
            if !record.annotations.is_object() {
                record.annotations = json!({});
            }
            if ack.item.is_empty() {
                record.annotations[key] = json!(ack.cursor);
            } else {
                if !record.annotations[key].is_object() {
                    record.annotations[key] = json!({});
                }
                record.annotations[key][&ack.item] = json!(true);
            }
        };
        if persist {
            let mut record = self
                .get_sync(&ack.execution_id)?
                .ok_or_else(|| anyhow::anyhow!("ack execution missing"))?;
            apply(&mut record);
            self.save(record)?;
        } else {
            let record = self
                .records
                .get_mut(&ack.execution_id)
                .expect("ack execution");
            apply(record);
            self.revision = self
                .revision
                .checked_add(1)
                .expect("journal revision exhausted");
            self.summaries
                .get_mut(&ack.execution_id)
                .expect("ack summary")
                .revision = self.revision;
            // Only terminal confirmation changes membership; root reports are
            // selected from the transactional database queue.
            if ack.action == "layered_terminal" && !super::pending::needs_terminal(record) {
                self.terminal_candidates.remove(&ack.execution_id);
            }
        }
        Ok(())
    }
}
