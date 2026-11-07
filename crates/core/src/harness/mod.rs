//! Execution selection and private, durable harness state.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, str::FromStr};
pub mod scope;
mod settings;
pub use settings::CodexSettings;
mod remote;
mod runtime;
pub use remote::{RemoteSession, ServerCapability, ServerConnection};
pub use runtime::{agent_settings, pin_agent_settings, RuntimeSettings, Versioned};

/// One-shot remote subtask registered on the parent session's runtime.
/// Pure UI bookkeeping: the server owns execution truth; this record only
/// lets the parent session badge, append and cost-fold finished subtasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtaskRef {
    /// Server execution id (also the local bookmark session id).
    pub execution_id: String,
    /// Execution kind as its protocol string (e.g. "agent", "todos").
    pub kind: String,
    /// Registered capability id the subtask was dispatched to.
    pub capability_id: String,
    /// The one-shot question submitted with the dispatch.
    pub question: String,
    /// Lifecycle status: "pending" | "running" | "done" | "error" | "cancelled".
    pub status: String,
    /// Bounded result summary fetched at completion (may be None).
    #[serde(default)]
    pub result_summary: Option<String>,
    /// Aggregated sub-execution token usage folded into the parent cost.
    #[serde(default)]
    pub tokens: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    #[default]
    Opencoder,
    Codex,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opencoder => "opencoder",
            Self::Codex => "codex",
        }
    }
}
impl FromStr for Harness {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opencoder" => Ok(Self::Opencoder),
            "codex" => Ok(Self::Codex),
            _ => Err(format!(
                "unknown harness '{s}'; expected opencoder or codex"
            )),
        }
    }
}

/// Private state: never serialize this object into public session responses.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessRuntime {
    /// Frontend input policy, pinned with the session.
    pub literal_mentions: bool,
    /// TUI task binding only; no credentials or remote harness overrides.
    pub remote: Option<RemoteSession>,
    pub codex: Option<CodexSettings>,
    pub harness: Harness,
    pub thread_id: Option<String>,
    pub fork_from: Option<String>,
    pub resource_root: Option<PathBuf>,
    pub envs: BTreeMap<String, String>,
    /// Privilege-drop identity for tool subprocesses (Linux operator
    /// executions). `None` on non-Linux hosts and legacy runtimes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_as: Option<crate::platform::RunAsIdentity>,
    pub model: Option<String>,
    pub last_input_id: Option<String>,
    pub in_flight: bool,
    /// One-shot remote subtasks registered by this (parent) session. Serde
    /// default keeps old session records deserializable.
    #[serde(default)]
    pub subtasks: Vec<SubtaskRef>,
}

pub fn agent_harness(name: &str) -> Harness {
    crate::agent::read_agent_meta(name)
        .map(|m| m.harness)
        .unwrap_or_default()
}

/// Build fresh, frontend-independent harness state from an explicit selection
/// and launch environment. Thread and settings snapshots are pinned later by
/// the shared session runner when the first turn starts.
pub fn fresh_runtime(
    agent_default: Harness,
    selection: Option<Harness>,
    envs: BTreeMap<String, String>,
    model: Option<String>,
) -> HarnessRuntime {
    let harness = selection.unwrap_or(agent_default);
    HarnessRuntime {
        harness,
        envs,
        model: if harness == Harness::Codex {
            model
        } else {
            None
        },
        ..Default::default()
    }
}

/// A resumed session may contain managed profile variables in addition to
/// those supplied at launch. Explicitly supplied values must still match.
pub fn matches_requested_env(
    runtime: &HarnessRuntime,
    requested: &BTreeMap<String, String>,
) -> bool {
    requested
        .iter()
        .all(|(key, value)| runtime.envs.get(key) == Some(value))
}

/// Only fresh Codex sessions take managed defaults. Existing threads keep their snapshot.
pub fn pin_settings(runtime: &mut HarnessRuntime, settings: Option<&CodexSettings>) {
    if runtime.harness != Harness::Codex
        || runtime.codex.is_some()
        || runtime.thread_id.is_some()
        || runtime.last_input_id.is_some()
    {
        return;
    }
    if let Some(settings) = settings {
        let mut envs = settings.envs.clone();
        envs.extend(runtime.envs.clone());
        runtime.envs = envs;
        runtime.model = settings.model.clone();
        runtime.codex = Some(settings.clone());
    }
}

pub fn parse_env(s: &str) -> Result<(String, String), String> {
    let (key, value) = s.split_once('=').ok_or("environment must be KEY=VALUE")?;
    validate_env(key, value)?;
    Ok((key.into(), value.into()))
}

pub fn validate_env(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || key.contains(['=', '\0']) || value.contains('\0') {
        return Err("invalid environment variable name or value".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_preserves_payload_and_rejects_invalid_pairs() {
        assert_eq!(parse_env("A= x=y ").unwrap(), ("A".into(), " x=y ".into()));
        assert_eq!(parse_env("A=").unwrap().1, "");
        for s in ["A", "=x", "A=x\0"] {
            assert!(parse_env(s).is_err());
        }
    }
    #[test]
    fn old_runtime_defaults_to_native_and_unknown_harness_fails() {
        assert_eq!(
            serde_json::from_str::<HarnessRuntime>("{}")
                .unwrap()
                .harness,
            Harness::Opencoder
        );
        assert!(serde_json::from_str::<HarnessRuntime>(r#"{"harness":"typo"}"#).is_err());
    }
    #[test]
    fn runtime_subtasks_default_empty_and_ref_round_trips() {
        // Records persisted before subtasks existed stay deserializable.
        assert!(
            serde_json::from_str::<HarnessRuntime>(r#"{"in_flight":true}"#)
                .unwrap()
                .subtasks
                .is_empty()
        );
        let entry = SubtaskRef {
            execution_id: "agent-01".into(),
            kind: "agent".into(),
            capability_id: "ops".into(),
            question: "check?".into(),
            status: "done".into(),
            result_summary: None,
            tokens: 42,
            created_at: 7,
        };
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(serde_json::from_value::<SubtaskRef>(value).unwrap(), entry);
        // Old payloads without optional fields still decode with defaults.
        assert_eq!(
            serde_json::from_str::<SubtaskRef>(
                r#"{"execution_id":"a","kind":"agent","capability_id":"c","question":"q",
                    "status":"pending","created_at":1}"#
            )
            .unwrap()
            .tokens,
            0
        );
    }
    #[test]
    fn fresh_runtime_keeps_explicit_selection_and_literal_environment() {
        let envs = BTreeMap::from([("NOTE".into(), "literal $(value)".into())]);
        let runtime = fresh_runtime(
            Harness::Opencoder,
            Some(Harness::Codex),
            envs.clone(),
            Some("codex-model".into()),
        );
        assert_eq!(runtime.harness, Harness::Codex);
        assert_eq!(runtime.envs, envs);
        assert_eq!(runtime.model.as_deref(), Some("codex-model"));
        assert!(runtime.thread_id.is_none());
        assert!(runtime.codex.is_none());
        assert!(
            runtime.run_as.is_none(),
            "fresh runtimes never inherit an identity"
        );
        assert_eq!(
            fresh_runtime(Harness::Opencoder, None, BTreeMap::new(), Some("x".into())).model,
            None
        );
    }
    #[test]
    fn run_as_round_trips_and_legacy_runtimes_deserialize_without_it() {
        let runtime = HarnessRuntime {
            run_as: Some(crate::platform::RunAsIdentity {
                uid: 950,
                gid: 950,
                groups: vec![950],
                username: Some("ocop-operator-fixture".into()),
            }),
            ..fresh_runtime(Harness::Opencoder, None, BTreeMap::new(), None)
        };
        let persisted = serde_json::to_string(&runtime).unwrap();
        assert!(persisted.contains(r#""run_as":{"uid":950"#));
        assert_eq!(
            serde_json::from_str::<HarnessRuntime>(&persisted)
                .unwrap()
                .run_as,
            runtime.run_as
        );
        // Legacy persisted runtimes (and the empty form) carry no run_as.
        let legacy = r#"{"literal_mentions":true,"envs":{"HOME":"/node/home"}}"#;
        assert_eq!(
            serde_json::from_str::<HarnessRuntime>(legacy)
                .unwrap()
                .run_as,
            None
        );
        assert_eq!(
            serde_json::from_str::<HarnessRuntime>("{}").unwrap().run_as,
            None
        );
        // Round-tripping drops the field entirely when unset.
        assert!(!serde_json::to_string(&HarnessRuntime::default())
            .unwrap()
            .contains("run_as"));
    }
    #[test]
    fn resume_environment_matches_requested_values_with_managed_defaults() {
        let runtime = fresh_runtime(
            Harness::Codex,
            None,
            BTreeMap::from([
                ("CODEX_HOME".into(), "/auth".into()),
                ("MANAGED".into(), "default".into()),
            ]),
            None,
        );
        assert!(matches_requested_env(
            &runtime,
            &BTreeMap::from([("CODEX_HOME".into(), "/auth".into())])
        ));
        assert!(!matches_requested_env(
            &runtime,
            &BTreeMap::from([("CODEX_HOME".into(), "/other".into())])
        ));
    }
}
