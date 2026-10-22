//! Keep history unrelated to Brain out of the periodic report path.
use super::*;
use crate::brain::v4::parent;
mod acknowledgements;
mod pending;
pub(crate) use pending::DispatchCache;

impl Journal {
    pub(super) fn update_report_candidate(&mut self, record: &Record) {
        let input = &record.assignment.request.input;
        let id = &record.assignment.index.id;
        self.update_terminal_candidate(record);
        if parent::reports(input)
            || (record.assignment.index.kind == ExecutionKind::Brain
                && input["schema_version"] == 7)
        {
            self.report_candidates.insert(id.clone());
        } else {
            self.report_candidates.remove(id);
        }
    }
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod tests;
