use std::collections::BTreeMap;

use serde_json::Value;

use crate::event::LlmEvent;

#[derive(Debug, Clone, Default)]
pub struct PartialTool {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct CompletedToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Default)]
pub struct ToolAccumulator {
    pub tools: BTreeMap<usize, PartialTool>,
}

impl ToolAccumulator {
    pub fn apply(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> Vec<LlmEvent> {
        let mut events = Vec::new();
        let existed = self.tools.contains_key(&index);
        let entry = self.tools.entry(index).or_default();
        let mut just_started = false;
        if let Some(i) = id {
            if entry.id.is_empty() {
                entry.id = i.to_string();
            }
        }
        if let Some(n) = name {
            if entry.name.is_empty() {
                entry.name = n.to_string();
            }
        }
        if !existed {
            just_started = true;
        }
        if let Some(a) = arguments {
            entry.arguments.push_str(a);
        }
        if just_started && !entry.id.is_empty() && !entry.name.is_empty() {
            events.push(LlmEvent::ToolCallStart {
                index,
                id: entry.id.clone(),
                name: entry.name.clone(),
            });
        }
        if let Some(a) = arguments {
            if !a.is_empty() {
                events.push(LlmEvent::ToolCallDelta {
                    index,
                    arguments: a.to_string(),
                });
            }
        }
        events
    }

    pub fn finish_all(&mut self) -> anyhow::Result<Vec<CompletedToolCall>> {
        let mut out = Vec::new();
        for (_, partial) in std::mem::take(&mut self.tools).into_iter() {
            let input: Value = if partial.arguments.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&partial.arguments).unwrap_or_else(|_| {
                    Value::Object(serde_json::Map::from_iter([(
                        "_raw_arguments".to_string(),
                        Value::String(partial.arguments.clone()),
                    )]))
                })
            };
            out.push(CompletedToolCall {
                id: partial.id,
                name: partial.name,
                input,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_emits_start_on_first_seen_then_delta() {
        let mut acc = ToolAccumulator::default();
        // First call with id+name → emits ToolCallStart (just_created + id/name set)
        let evs1 = acc.apply(0, Some("call_1"), Some("bash"), None);
        assert!(evs1.iter().any(|e| matches!(
            e,
            LlmEvent::ToolCallStart { id, name, .. } if id == "call_1" && name == "bash"
        )), "expected ToolCallStart in {:?}", evs1);
        assert!(!evs1.iter().any(|e| matches!(e, LlmEvent::ToolCallDelta { .. })));
        // Second call with same index + args → only Delta (already started)
        let evs2 = acc.apply(0, Some("call_1"), Some("bash"), Some("{\"cmd\":"));
        assert!(!evs2.iter().any(|e| matches!(e, LlmEvent::ToolCallStart { .. })));
        assert!(evs2.iter().any(|e| matches!(
            e,
            LlmEvent::ToolCallDelta { arguments, .. } if arguments == "{\"cmd\":"
        )), "expected ToolCallDelta in {:?}", evs2);
    }

    #[test]
    fn finish_all_parses_json_and_fallback_on_invalid() {
        let mut acc = ToolAccumulator::default();
        acc.apply(0, Some("c1"), Some("edit"), Some("{\"path\":\"a.txt\"}"));
        acc.apply(1, Some("c2"), Some("bash"), Some("not valid json"));
        let calls = acc.finish_all().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].input["path"], "a.txt");
        assert_eq!(calls[1].input["_raw_arguments"], "not valid json");
    }

    #[test]
    fn finish_all_empty_args_yields_empty_object() {
        let mut acc = ToolAccumulator::default();
        acc.apply(0, Some("c1"), Some("ls"), None);
        let calls = acc.finish_all().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].input.as_object().unwrap().is_empty());
    }
}
