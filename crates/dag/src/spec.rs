use serde::{Deserialize, Serialize};

pub const MAX_HOW_APPEND_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DagSpec {
    #[serde(default = "crate::policies::default_concurrency")]
    pub max_concurrency: usize,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub steps: Vec<StepSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub trigger_rule: TriggerRule,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepKind {
    Dynamic {
        #[serde(default)]
        failure_policy: FailurePolicy,
        source: crate::dynamic::DynamicSource,
        template: Box<StepKind>,
    },
    Agent {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        how_append: Option<String>,
    },
    Binary {
        resource: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TriggerRule {
    #[default]
    AllSuccess,
    AllDone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    #[default]
    FailFast,
    CollectAll,
}

pub fn decode_spec(value: &serde_json::Value) -> Result<DagSpec, String> {
    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
}

pub fn decode_spec_str(raw: &str) -> Result<DagSpec, String> {
    serde_json::from_str(raw).map_err(|error| error.to_string())
}

impl StepKind {
    pub fn executable(&self) -> &Self {
        match self {
            Self::Dynamic { template, .. } => template,
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_spec_preserves_argument_boundaries_and_pins() {
        let value = json!({"name":"files","steps":[{"name":"read","kind":{
            "type":"binary","resource":"reader@v3","args":["a b","\"quoted\"",""]
        }},{"name":"review","depends_on":["read"],"kind":{
            "type":"agent","prompt":"review","how_append":"local"
        }}]});
        let spec = decode_spec(&value).unwrap();
        assert_eq!(spec.max_concurrency, 4);
        assert_eq!(
            spec.steps[0].kind,
            StepKind::Binary {
                resource: "reader@v3".into(),
                args: vec!["a b".into(), "\"quoted\"".into(), "".into()]
            }
        );
        assert_eq!(
            decode_spec_str(&serde_json::to_string(&spec).unwrap()).unwrap(),
            spec
        );
    }

    #[test]
    fn invalid_types_fields_and_string_arguments_are_rejected() {
        for kind in [
            json!({"type":"unknown","resource":"tool"}),
            json!({"type":"binary","resource":"tool","sandbox":"host"}),
            json!({"type":"binary","resource":"tool","args":"a b"}),
        ] {
            assert!(
                decode_spec(&json!({"name":"invalid","steps":[{"name":"a","kind":kind}]})).is_err()
            );
        }
        assert!(decode_spec_str("{invalid").is_err());
    }
}
