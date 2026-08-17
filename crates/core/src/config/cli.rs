//! User-registered CLI instructions injected into the model system prompt.

use std::fmt;

use crate::agent::AgentMode;
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which agent sessions receive a registered integration (multi-select).
///
/// `parent` covers every primary agent (act/plan/command); `explore`/`build`
/// target the read-only / implementation subagents by agent name. Serialized
/// as an array of selected tags (e.g. `["explore","build"]`); legacy
/// single-string values (`"parent"` / `"subagents"` / `"all"`) still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectionTarget {
    pub parent: bool,
    pub explore: bool,
    pub build: bool,
}

impl InjectionTarget {
    pub const fn parent_only() -> Self {
        Self {
            parent: true,
            explore: false,
            build: false,
        }
    }

    /// Legacy `subagents` scope: both subagents, no primary agent.
    pub const fn subagents() -> Self {
        Self {
            parent: false,
            explore: true,
            build: true,
        }
    }

    pub const fn all() -> Self {
        Self {
            parent: true,
            explore: true,
            build: true,
        }
    }

    const fn none() -> Self {
        Self {
            parent: false,
            explore: false,
            build: false,
        }
    }

    /// The serde default (`parent`-only) — used to omit the field on save.
    pub const fn is_parent_default(&self) -> bool {
        self.parent && !self.explore && !self.build
    }

    /// Does the agent session named `name`, running in `mode`, receive this
    /// entry? Primary agents all share the `parent` flag; subagents are
    /// matched by their fixed names (`explore` / `build`).
    pub fn allows_agent(self, name: &str, mode: AgentMode) -> bool {
        match mode {
            AgentMode::Primary => self.parent,
            AgentMode::Subagent => match name {
                "explore" => self.explore,
                "build" => self.build,
                _ => false,
            },
        }
    }

    /// Compact display label, e.g. `parent+explore` or `none`.
    pub fn label(self) -> String {
        let mut tags: Vec<&str> = Vec::new();
        if self.parent {
            tags.push("parent");
        }
        if self.explore {
            tags.push("explore");
        }
        if self.build {
            tags.push("build");
        }
        if tags.is_empty() {
            "none".to_string()
        } else {
            tags.join("+")
        }
    }

    fn apply_tag(&mut self, tag: &str) {
        match tag {
            "parent" => self.parent = true,
            "explore" => self.explore = true,
            "build" => self.build = true,
            // legacy single-string aliases
            "subagents" => *self = Self::subagents(),
            "all" => *self = Self::all(),
            _ => {}
        }
    }
}

impl Default for InjectionTarget {
    fn default() -> Self {
        Self::parent_only()
    }
}

impl Serialize for InjectionTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut tags: Vec<&str> = Vec::new();
        if self.parent {
            tags.push("parent");
        }
        if self.explore {
            tags.push("explore");
        }
        if self.build {
            tags.push("build");
        }
        tags.serialize(serializer)
    }
}

struct TargetVisitor;

impl<'de> de::Visitor<'de> for TargetVisitor {
    type Value = InjectionTarget;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an injection target tag, tag array, or legacy string")
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
        let mut target = InjectionTarget::none();
        target.apply_tag(s);
        Ok(target)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut target = InjectionTarget::none();
        while let Some(tag) = seq.next_element::<String>()? {
            target.apply_tag(&tag);
        }
        Ok(target)
    }
}

impl<'de> Deserialize<'de> for InjectionTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TargetVisitor)
    }
}

/// One named CLI registration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliConfig {
    /// Disabled registrations remain editable but are not sent to the model.
    #[serde(default)]
    pub enabled: bool,
    /// Agent sessions that receive this registration.
    #[serde(default, skip_serializing_if = "InjectionTarget::is_parent_default")]
    pub inject_to: InjectionTarget,
    /// Free-form usage contract for this CLI (commands, constraints, examples).
    #[serde(default)]
    pub content: String,
}

/// Parse an `inject_to` patch value (tag, tag array, or legacy string).
fn parse_patch_target(value: &serde_json::Value) -> Option<InjectionTarget> {
    serde_json::from_value::<InjectionTarget>(value.clone()).ok()
}

/// Merge a JSON patch into a CLI registration without clearing omitted fields.
pub(super) fn merge(cfg: &mut CliConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(enabled) = obj.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = enabled;
    }
    if let Some(target) = obj.get("inject_to").and_then(parse_patch_target) {
        cfg.inject_to = target;
    }
    if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
        cfg.content = content.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_tag_array() {
        let json = serde_json::to_string(&InjectionTarget::subagents()).unwrap();
        assert_eq!(json, r#"["explore","build"]"#);
        let json = serde_json::to_string(&InjectionTarget::all()).unwrap();
        assert_eq!(json, r#"["parent","explore","build"]"#);
    }

    #[test]
    fn parent_only_is_default_and_roundtrips() {
        let t = InjectionTarget::default();
        assert_eq!(t, InjectionTarget::parent_only());
        assert!(t.is_parent_default());
        let back: InjectionTarget = serde_json::from_str(r#"["parent"]"#).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn loads_legacy_string_values() {
        let parent: InjectionTarget = serde_json::from_str("\"parent\"").unwrap();
        assert_eq!(parent, InjectionTarget::parent_only());
        let subs: InjectionTarget = serde_json::from_str("\"subagents\"").unwrap();
        assert_eq!(subs, InjectionTarget::subagents());
        assert!(!subs.parent);
        let all: InjectionTarget = serde_json::from_str("\"all\"").unwrap();
        assert_eq!(all, InjectionTarget::all());
    }

    #[test]
    fn loads_tag_arrays_and_ignores_unknown_tags() {
        let t: InjectionTarget = serde_json::from_str(r#"["explore","bogus"]"#).unwrap();
        assert!(t.explore);
        assert!(!t.parent);
        assert!(!t.build);
    }

    #[test]
    fn cli_config_omits_parent_only_inject_to() {
        let cfg = CliConfig::default();
        let json = serde_json::to_value(&cfg).unwrap();
        assert!(json.get("inject_to").is_none());
        let back: CliConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back.inject_to, InjectionTarget::parent_only());
    }

    #[test]
    fn cli_config_roundtrips_multi_target() {
        let cfg = CliConfig {
            enabled: true,
            inject_to: InjectionTarget {
                parent: false,
                explore: true,
                build: true,
            },
            content: "use foo".into(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["inject_to"], serde_json::json!(["explore", "build"]));
        let back: CliConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back.inject_to, cfg.inject_to);
    }

    #[test]
    fn merge_accepts_string_and_array_inject_to() {
        let mut cfg = CliConfig::default();
        merge(
            &mut cfg,
            serde_json::json!({ "inject_to": "subagents" })
                .as_object()
                .unwrap(),
        );
        assert_eq!(cfg.inject_to, InjectionTarget::subagents());
        merge(
            &mut cfg,
            serde_json::json!({ "inject_to": ["parent", "build"] })
                .as_object()
                .unwrap(),
        );
        assert!(cfg.inject_to.parent);
        assert!(!cfg.inject_to.explore);
        assert!(cfg.inject_to.build);
    }

    #[test]
    fn allows_agent_matrix() {
        let parent_only = InjectionTarget::parent_only();
        assert!(parent_only.allows_agent("act", AgentMode::Primary));
        assert!(parent_only.allows_agent("plan", AgentMode::Primary));
        assert!(!parent_only.allows_agent("explore", AgentMode::Subagent));
        assert!(!parent_only.allows_agent("build", AgentMode::Subagent));

        let explore_only = InjectionTarget {
            parent: false,
            explore: true,
            build: false,
        };
        assert!(!explore_only.allows_agent("act", AgentMode::Primary));
        assert!(explore_only.allows_agent("explore", AgentMode::Subagent));
        assert!(!explore_only.allows_agent("build", AgentMode::Subagent));

        let build_only = InjectionTarget {
            parent: false,
            explore: false,
            build: true,
        };
        assert!(build_only.allows_agent("build", AgentMode::Subagent));
        assert!(!build_only.allows_agent("explore", AgentMode::Subagent));
    }

    #[test]
    fn label_joins_selected_tags() {
        assert_eq!(InjectionTarget::parent_only().label(), "parent");
        assert_eq!(
            InjectionTarget {
                parent: false,
                explore: true,
                build: true
            }
            .label(),
            "explore+build"
        );
        assert_eq!(InjectionTarget::all().label(), "parent+explore+build");
        assert_eq!(InjectionTarget::none().label(), "none");
    }
}
