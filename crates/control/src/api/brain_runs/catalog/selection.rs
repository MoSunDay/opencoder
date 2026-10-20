use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    Builtin,
    Stored,
    Generated,
    Plan,
}

pub(super) struct Entry {
    pub origin: Origin,
    pub value: Value,
}

impl Entry {
    pub fn new(origin: Origin, value: Value) -> Self {
        Self { origin, value }
    }
}

// Invalid contracts are never treated as interchangeable. Missing fields
// mean an empty contract, matching catalog/admission defaults.
fn contract<'a>(value: &'a Value, field: &str) -> Option<BTreeSet<&'a str>> {
    match value.get(field) {
        None => Some(BTreeSet::new()),
        Some(Value::Array(fields)) => fields.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

fn equivalent(a: &Value, b: &Value) -> bool {
    a["kind"].is_string()
        && a["target"].is_string()
        && a["definition"].is_object()
        && a["kind"] == b["kind"]
        && a["target"] == b["target"]
        && a["definition"] == b["definition"]
        && ["required_inputs", "required_outputs"].iter().all(|field| {
            let left = contract(a, field);
            left.is_some() && left == contract(b, field)
        })
}

pub(super) fn ids(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| entry.value["unavailable_reason"].is_null())
        .filter(|entry| {
            entry.origin != Origin::Generated
                || !entries.iter().any(|stored| {
                    stored.origin == Origin::Stored && equivalent(&entry.value, &stored.value)
                })
        })
        .filter_map(|entry| entry.value["id"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, origin: Origin) -> Entry {
        Entry::new(
            origin,
            json!({"id":id,"kind":"agent","target":"worker","definition":{"prompt":"task"}}),
        )
    }

    #[test]
    fn only_generated_equivalents_are_hidden_and_all_stored_ids_survive() {
        let entries = vec![
            entry("generated-looking", Origin::Stored),
            entry("second", Origin::Stored),
            entry("arbitrary-id", Origin::Generated),
            entry("builtin", Origin::Builtin),
            entry("plan", Origin::Plan),
        ];
        assert_eq!(
            ids(&entries),
            ["generated-looking", "second", "builtin", "plan"]
        );
        assert_eq!(entries[2].value["id"], "arbitrary-id");
    }

    #[test]
    fn distinct_definitions_kinds_targets_and_contracts_remain_selectable() {
        for (field, value) in [
            ("kind", json!("operator")),
            ("target", json!("other")),
            ("definition", json!({"prompt":"different"})),
            ("definition", json!({"plan_version":2})),
            ("required_inputs", json!(["source"])),
            ("required_outputs", json!(["result"])),
            ("required_inputs", Value::Null),
        ] {
            let mut generated = entry("generated", Origin::Generated);
            generated.value[field] = value;
            assert_eq!(
                ids(&[entry("stored", Origin::Stored), generated]).len(),
                2,
                "{field}"
            );
        }
    }

    #[test]
    fn contracts_are_sets_and_unresolved_definitions_are_not_merged() {
        let mut stored = entry("stored", Origin::Stored);
        let mut generated = entry("generated", Origin::Generated);
        stored.value["required_inputs"] = json!(["a", "b", "a"]);
        generated.value["required_inputs"] = json!(["b", "a"]);
        generated.value["required_outputs"] = json!([]);
        assert_eq!(ids(&[stored, generated]), ["stored"]);
        let mut stored = entry("stored", Origin::Stored);
        let mut generated = entry("generated", Origin::Generated);
        stored.value["definition"] = Value::Null;
        generated.value["definition"] = Value::Null;
        assert_eq!(ids(&[stored, generated]).len(), 2);
    }
}
