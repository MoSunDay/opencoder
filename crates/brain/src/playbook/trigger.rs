//! Trigger matching for playbooks — the pure half of the brain's
//! dual-track scheduling (`manual dispatch` + `auto-fire on inbound
//! message`). Everything here is a pure function over a
//! [`PlaybookTrigger`] plus precomputed similarities: the embedding itself
//! happens in the caller, so trigger semantics are unit-testable with no
//! store and no LLM (the same split `plan.rs` uses for the routing walk).
//!
//! Fail-closed is the contract: a `Manual` trigger never fires, and a
//! `Message` trigger without a similarity (embed outage upstream) never
//! fires either — an inbound message can silently skip a playbook, never
//! silently start one.

use opencoder_store::BrainPlaybookRecord;

use super::spec::{PlaybookSpec, PlaybookTrigger};

/// The text a `Message` trigger matches against: trimmed `match_text`, or
/// `None` for a `Manual` trigger (or an effectively-empty match text — a
/// blank pattern cannot embed, so it can never fire).
pub fn match_text(trigger: &PlaybookTrigger) -> Option<&str> {
    match trigger {
        PlaybookTrigger::Message { match_text, .. } => {
            let trimmed = match_text.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        }
        PlaybookTrigger::Manual {} => None,
    }
}

/// The cosine cut a `Message` trigger fires at. `Manual` reports infinity —
/// finite similarities never reach it, which is exactly `Manual` never
/// firing expressed as a number.
pub fn threshold(trigger: &PlaybookTrigger) -> f64 {
    match trigger {
        PlaybookTrigger::Message { threshold, .. } => *threshold,
        PlaybookTrigger::Manual {} => f64::INFINITY,
    }
}

/// Whether one trigger fires at `similarity` (the cosine between the
/// embedded `match_text` and the embedded inbound text). `None` similarity
/// — an embed failure upstream — never fires (fail-closed).
pub fn fires(trigger: &PlaybookTrigger, similarity: Option<f64>) -> bool {
    match trigger {
        PlaybookTrigger::Manual {} => false,
        PlaybookTrigger::Message { threshold, .. } => {
            similarity.is_some_and(|score| score >= *threshold)
        }
    }
}

/// Filter persisted playbook records down to the ones whose trigger fires,
/// given a similarity lookup computed by the caller (who embeds each
/// `Message` trigger's [`match_text`] and cosines it against the inbound
/// text). Records whose `spec_json` does not parse are skipped — a corrupt
/// row must not take the scan down.
pub fn scan(
    records: &[BrainPlaybookRecord],
    similarity_of: impl Fn(&BrainPlaybookRecord) -> Option<f64>,
) -> Vec<BrainPlaybookRecord> {
    records
        .iter()
        .filter(|record| {
            serde_json::from_str::<PlaybookSpec>(&record.spec_json)
                .map(|spec| fires(&spec.trigger, similarity_of(record)))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Cosine similarity as a plain `Option` — the same arithmetic as the plan
/// tree's walk (`plan::cosine`), with dimension mismatches and zero norms
/// folded into `None` so trigger callers never handle a `Result`.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f64> {
    crate::plan::cosine(a, b).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playbook::{PlaybookOrigin, PlaybookStep, PlaybookTarget, SCHEMA_VERSION};

    fn message(match_text: &str, threshold: f64) -> PlaybookTrigger {
        PlaybookTrigger::Message {
            match_text: match_text.into(),
            threshold,
        }
    }

    fn record(
        id: &str,
        trigger: &PlaybookTrigger,
        spec_json: Option<String>,
    ) -> BrainPlaybookRecord {
        BrainPlaybookRecord {
            id: id.into(),
            name: id.into(),
            origin: "fixed".into(),
            situation_digest: None,
            spec_json: spec_json.unwrap_or_else(|| {
                serde_json::to_string(&PlaybookSpec {
                    schema_version: SCHEMA_VERSION,
                    id: id.into(),
                    name: id.into(),
                    origin: PlaybookOrigin::Fixed {},
                    trigger: trigger.clone(),
                    steps: vec![PlaybookStep {
                        name: "act".into(),
                        depends_on: Vec::new(),
                        target: PlaybookTarget::Agent {
                            agent: "act".into(),
                        },
                        prompt: "do {situation}".into(),
                    }],
                })
                .unwrap()
            }),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn manual_never_fires() {
        let manual = PlaybookTrigger::Manual {};
        assert!(match_text(&manual).is_none());
        assert_eq!(threshold(&manual), f64::INFINITY);
        assert!(!fires(&manual, Some(1.0)));
        assert!(!fires(&manual, Some(f64::INFINITY)));
        assert!(!fires(&manual, None));
    }

    #[test]
    fn message_fires_at_or_above_threshold_only() {
        let trigger = message("deploy the db", 0.8);
        assert_eq!(match_text(&trigger), Some("deploy the db"));
        assert_eq!(threshold(&trigger), 0.8);
        assert!(fires(&trigger, Some(0.8)));
        assert!(fires(&trigger, Some(0.99)));
        assert!(!fires(&trigger, Some(0.79)));
        assert!(!fires(&trigger, None), "embed failure is fail-closed");
    }

    #[test]
    fn match_text_trims_and_rejects_blank_patterns() {
        assert_eq!(match_text(&message("  x  ", 0.5)), Some("x"));
        assert_eq!(match_text(&message("   ", 0.5)), None);
    }

    #[test]
    fn scan_filters_corrupt_and_manual_records() {
        let hot = record("hot", &message("resize the fleet", 0.9), None);
        let cold = record("cold", &message("unrelated topic", 0.9), None);
        let manual = record("manual", &PlaybookTrigger::Manual {}, None);
        let corrupt = BrainPlaybookRecord {
            spec_json: "{not json".into(),
            ..record("corrupt", &PlaybookTrigger::Manual {}, None)
        };
        let records = vec![hot.clone(), cold, manual, corrupt];
        let matched = scan(&records, |record| (record.id == "hot").then_some(1.0));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "hot");
        // Fail-closed: a None similarity (embed failure) skips everything.
        assert!(scan(&records, |_| None).is_empty());
    }

    #[test]
    fn cosine_similarity_is_none_for_nan_and_zero_vectors() {
        // NaN components fold into the norms and the shared cosine guard
        // rejects them, so poisoned embeddings can never fire a trigger.
        assert!(cosine_similarity(&[f32::NAN, 1.0], &[1.0, 0.0]).is_none());
        assert!(cosine_similarity(&[1.0, 0.0], &[f32::NAN, 1.0]).is_none());
        assert!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]).is_none());
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
    }
}
