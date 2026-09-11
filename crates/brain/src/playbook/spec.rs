//! Playbook wire types, validation and prompt rendering — the pure half of
//! the playbook domain (topology helpers live in [`super::topology`]).
//!
//! Every type here is serde-tagged (`kind`) exactly the way the planner
//! prompt mandates, so an LLM's raw JSON parses straight into a
//! [`PlaybookInput`]; the runtime then mints the id and origin into a
//! [`PlaybookSpec`], and [`validate`] enforces the whole contract in one
//! aggregated pass (mirroring `opencoder_dag::validate`'s `Vec<String>`
//! style) before anything is persisted or executed.

use serde::{Deserialize, Serialize};

use super::topology;

/// Wire-format revision of the playbook spec. Bumped whenever the persisted
/// shape changes in a way old rows cannot be re-read as.
pub const SCHEMA_VERSION: u32 = 1;
/// Hard ceiling on step count (the planner prompt asks for ≤ 8; this is the
/// validator's backstop so a rambling model can never build a maze).
pub const MAX_STEPS: usize = 64;
/// Longest accepted dependency chain (prompt asks for ≤ 6; backstop).
pub const MAX_CHAIN_DEPTH: usize = 16;
/// Widest accepted ready batch — the most steps whose dependencies are all
/// done at once (a fan-out guard: executors run one batch concurrently).
pub const MAX_WIDTH: usize = 16;
/// Longest step name, in bytes of the trimmed string. Step names are slugs
/// (ASCII lowercase alphanumerics plus `-`/`_`) because they double as map
/// keys in run-state and event payloads.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest playbook name, in bytes of the trimmed string.
pub const MAX_PLAYBOOK_NAME_CHARS: usize = 120;
/// Longest step prompt template, in chars (the `{situation}` placeholder is
/// substituted at dispatch time; unknown placeholders pass through).
pub const MAX_PROMPT_CHARS: usize = 8000;
/// Longest spec id, in bytes — runtime-minted `playbook-{ULID}` ids sit far
/// under this; the ceiling is the backstop against injected junk.
pub const MAX_ID_CHARS: usize = 256;
/// Longest target reference (agent/team/dag/workflow/capability_id/route
/// ref), in bytes — every one of these flows into executor dispatches, so
/// they share one injection ceiling.
pub const MAX_TARGET_REF_CHARS: usize = 256;
/// Longest trigger `match_text`, in bytes — the text gets embedded (and
/// echoed through trigger payloads), so it carries its own ceiling.
pub const MAX_MATCH_TEXT_CHARS: usize = 2000;

/// A validated playbook — the orchestration graph. `id` is minted by the
/// runtime (`playbook-{ULID}`); the store keeps the serialized spec opaque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookSpec {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub origin: PlaybookOrigin,
    pub trigger: PlaybookTrigger,
    pub steps: Vec<PlaybookStep>,
}

/// How the playbook came to be. Tagged serde (`kind`: `fixed` | `dynamic`)
/// is the exact wire shape the planner prompt mandates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybookOrigin {
    /// Authored by hand (`{"kind":"fixed"}`) — an empty struct variant,
    /// which is the shape internally-tagged unit variants deserialize as.
    Fixed {},
    /// Minted by the LLM planner for one situation; `situation_digest` is
    /// the plan-cache reuse key, `plan_id` the decision tree that routed
    /// here (provenance, optional).
    Dynamic {
        situation_digest: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_id: Option<String>,
    },
}

/// When the playbook fires. Tagged serde (`kind`: `manual` | `message`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybookTrigger {
    /// Started explicitly by a human (`{"kind":"manual"}`).
    Manual {},
    /// Auto-fired when an incoming message matches `match_text` with
    /// similarity ≥ `threshold` (cosine, 0.0..=1.0).
    Message { match_text: String, threshold: f64 },
}

impl Default for PlaybookTrigger {
    fn default() -> Self {
        PlaybookTrigger::Manual {}
    }
}

/// One step of the graph: a target executor plus the prompt template it
/// runs under. `depends_on` names sibling steps that must finish first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookStep {
    pub name: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub target: PlaybookTarget,
    pub prompt: String,
}

/// Which executor a step hands its prompt to. Tagged serde (`kind`:
/// `agent` | `team` | `dag` | `todos` | `brain`) — the wire contract the
/// planner prompt mandates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybookTarget {
    Agent {
        agent: String,
    },
    Team {
        team: String,
    },
    Dag {
        dag: String,
    },
    Todos {
        workflow: String,
    },
    /// Resolve `capability_id` through the owning environment's binding
    /// (control-plane `capability_target`, local brain routes) — unless the
    /// optional inline `route` pins one executor for every end.
    Brain {
        capability_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        route: Option<PlaybookRoute>,
    },
}

/// Inline executor route carried by a brain step target — the cross-end
/// determinism channel: when present, both the local executor and the
/// control-plane dispatch resolve the capability to exactly this executor,
/// so one spec cannot diverge through environment-specific bindings.
/// `todos`/`brain` routes are excluded at the type level (platform-only /
/// nesting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybookRouteKind {
    Agent,
    Team,
    Dag,
}

/// The executor a routed brain step hands off to (`{"kind":"team","ref":"x"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookRoute {
    pub kind: PlaybookRouteKind,
    #[serde(rename = "ref")]
    pub ref_: String,
}

/// Payload for creating (or wholly rewriting) a playbook — everything but
/// the runtime-minted `id` and the origin the runtime attaches.
#[derive(Debug, Clone, Deserialize)]
pub struct PlaybookInput {
    pub name: String,
    #[serde(default)]
    pub trigger: PlaybookTrigger,
    pub steps: Vec<PlaybookStep>,
}

/// Validate a spec. Every rule names the offending field in its message and
/// ALL problems are collected (aggregated, not first-only) so dispatch
/// rejects with a complete report. Acyclicity, chain depth and fan-out
/// width are delegated to [`topology`].
pub fn validate(spec: &PlaybookSpec) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    if spec.schema_version != SCHEMA_VERSION {
        errs.push(format!(
            "schema_version must be {SCHEMA_VERSION}, got {}",
            spec.schema_version
        ));
    }
    if spec.id.trim().is_empty() {
        errs.push("id must not be empty".to_string());
    } else if spec.id.len() > MAX_ID_CHARS {
        errs.push(format!(
            "id exceeds {MAX_ID_CHARS} bytes, got {}",
            spec.id.len()
        ));
    }
    let name_len = spec.name.trim().len();
    if name_len == 0 {
        errs.push("name must not be empty".to_string());
    } else if name_len > MAX_PLAYBOOK_NAME_CHARS {
        errs.push(format!(
            "name exceeds {MAX_PLAYBOOK_NAME_CHARS} bytes, got {name_len}"
        ));
    }
    if spec.steps.is_empty() {
        errs.push("steps must contain at least one step".to_string());
    }
    if spec.steps.len() > MAX_STEPS {
        errs.push(format!(
            "steps must be <= {MAX_STEPS}, got {}",
            spec.steps.len()
        ));
    }

    let names: Vec<&str> = spec.steps.iter().map(|s| s.name.as_str()).collect();
    for step in &spec.steps {
        if step.name.trim().is_empty() {
            errs.push(format!("step {:?} has an empty name", step.name));
        } else if !is_slug(&step.name) {
            errs.push(format!("step name {:?} is not a valid slug", step.name));
        }
        let mut seen_deps = std::collections::BTreeSet::new();
        for dep in &step.depends_on {
            if dep.trim().is_empty() {
                errs.push(format!("step {:?} depends on an empty name", step.name));
            } else if dep == &step.name {
                errs.push(format!("step {:?} depends on itself", step.name));
            } else if !names.contains(&dep.as_str()) {
                errs.push(format!(
                    "step {:?} depends on unknown step {:?}",
                    step.name, dep
                ));
            }
            if !seen_deps.insert(dep.as_str()) {
                errs.push(format!(
                    "step {:?} declares duplicate dependency {:?}",
                    step.name, dep
                ));
            }
        }
        let field = match &step.target {
            PlaybookTarget::Agent { agent } => Some(("agent", agent.as_str())),
            PlaybookTarget::Team { team } => Some(("team", team.as_str())),
            PlaybookTarget::Dag { dag } => Some(("dag", dag.as_str())),
            PlaybookTarget::Todos { workflow } => Some(("workflow", workflow.as_str())),
            PlaybookTarget::Brain {
                capability_id,
                route,
            } => {
                if let Some(route) = route {
                    let route_ref = route.ref_.as_str();
                    if route_ref.trim().is_empty() {
                        errs.push(format!(
                            "step {:?} brain route ref must not be empty",
                            step.name
                        ));
                    } else if route_ref.len() > MAX_TARGET_REF_CHARS {
                        errs.push(format!(
                            "step {:?} brain route ref exceeds {MAX_TARGET_REF_CHARS} bytes",
                            step.name
                        ));
                    }
                }
                Some(("capability_id", capability_id.as_str()))
            }
        };
        if let Some((field, value)) = field {
            if value.trim().is_empty() {
                errs.push(format!(
                    "step {:?} target {field} must not be empty",
                    step.name
                ));
            } else if value.len() > MAX_TARGET_REF_CHARS {
                errs.push(format!(
                    "step {:?} target {field} exceeds {MAX_TARGET_REF_CHARS} bytes",
                    step.name
                ));
            }
        }
        if step.prompt.trim().is_empty() {
            errs.push(format!("step {:?} has an empty prompt", step.name));
        } else if step.prompt.trim().chars().count() > MAX_PROMPT_CHARS {
            errs.push(format!(
                "step {:?} prompt exceeds {MAX_PROMPT_CHARS} chars",
                step.name
            ));
        }
    }
    // Duplicate names first: a duplicated name collapses the topology's
    // indegree map, so `topo_order` would report a bogus "cycle" on top of
    // the honest duplicate error — skip topology entirely in that case.
    let mut has_duplicate_names = false;
    for (i, n) in names.iter().enumerate() {
        if names[..i].contains(n) {
            errs.push(format!("duplicate step name {n:?}"));
            has_duplicate_names = true;
        }
    }
    match &spec.trigger {
        PlaybookTrigger::Manual {} => {}
        PlaybookTrigger::Message {
            match_text,
            threshold,
        } => {
            if match_text.trim().is_empty() {
                errs.push("trigger match_text must not be empty".to_string());
            } else if match_text.len() > MAX_MATCH_TEXT_CHARS {
                errs.push(format!(
                    "trigger match_text exceeds {MAX_MATCH_TEXT_CHARS} bytes"
                ));
            }
            if !threshold.is_finite() || !(0.0..=1.0).contains(threshold) {
                errs.push(format!(
                    "trigger threshold must be finite within 0.0..=1.0, got {threshold}"
                ));
            }
        }
    }

    if !has_duplicate_names {
        if let Err(msg) = topology::topo_order(spec) {
            errs.push(msg);
        } else {
            let depth = topology::chain_depth(spec);
            if depth > MAX_CHAIN_DEPTH {
                errs.push(format!(
                    "dependency chain depth {depth} exceeds {MAX_CHAIN_DEPTH}"
                ));
            }
            let width = topology::max_width(spec);
            if width > MAX_WIDTH {
                errs.push(format!("ready width {width} exceeds {MAX_WIDTH}"));
            }
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Validate a create/update payload exactly the way [`validate`] will once
/// the runtime mints the id and origin: build the draft spec the runtime
/// would persist (id "draft", origin `Fixed`, current schema version) and
/// run the full contract over it. Pure — the web layer calls this so a bad
/// payload is a clean 400 before any store write.
pub fn validate_draft(input: &PlaybookInput) -> Result<(), Vec<String>> {
    validate(&PlaybookSpec {
        schema_version: SCHEMA_VERSION,
        id: "draft".to_string(),
        name: input.name.trim().to_string(),
        origin: PlaybookOrigin::Fixed {},
        trigger: input.trigger.clone(),
        steps: input.steps.clone(),
    })
}

/// Slug rule for step names: non-empty, ≤ [`MAX_NAME_CHARS`] bytes, ASCII
/// lowercase alphanumerics plus `-` and `_` only.
pub fn is_slug(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Substitute the `{situation}` placeholder in a step prompt template with
/// the live situation text (trimmed). Unknown placeholders are left
/// verbatim — templates stay readable instead of failing dispatch.
pub fn render_prompt(template: &str, situation: &str) -> String {
    template.replace("{situation}", situation.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, steps: Vec<PlaybookStep>, trigger: PlaybookTrigger) -> PlaybookSpec {
        PlaybookSpec {
            schema_version: SCHEMA_VERSION,
            id: id.to_string(),
            name: "test".to_string(),
            origin: PlaybookOrigin::Fixed {},
            trigger,
            steps,
        }
    }

    fn step(name: &str, target: PlaybookTarget) -> PlaybookStep {
        PlaybookStep {
            name: name.to_string(),
            depends_on: Vec::new(),
            target,
            prompt: "handle {situation}".to_string(),
        }
    }

    fn agent(agent: &str) -> PlaybookTarget {
        PlaybookTarget::Agent {
            agent: agent.to_string(),
        }
    }

    fn message_trigger(match_text: &str) -> PlaybookTrigger {
        PlaybookTrigger::Message {
            match_text: match_text.to_string(),
            threshold: 0.8,
        }
    }

    #[test]
    fn empty_route_ref_is_rejected() {
        let errs = validate(&spec(
            "pb-1",
            vec![step(
                "act",
                PlaybookTarget::Brain {
                    capability_id: "cap-1".into(),
                    route: Some(PlaybookRoute {
                        kind: PlaybookRouteKind::Team,
                        ref_: "   ".into(),
                    }),
                },
            )],
            PlaybookTrigger::Manual {},
        ))
        .unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("brain route ref must not be empty")),
            "must name the empty route ref: {errs:?}"
        );
    }

    #[test]
    fn valid_route_passes_validation() {
        let result = validate(&spec(
            "pb-1",
            vec![step(
                "act",
                PlaybookTarget::Brain {
                    capability_id: "cap-1".into(),
                    route: Some(PlaybookRoute {
                        kind: PlaybookRouteKind::Team,
                        ref_: "auditors".into(),
                    }),
                },
            )],
            PlaybookTrigger::Manual {},
        ));
        assert!(result.is_ok(), "valid kind+ref route must pass: {result:?}");
    }

    #[test]
    fn oversize_fields_are_rejected() {
        let long_id = "i".repeat(MAX_ID_CHARS + 1);
        let errs = validate(&spec(
            &long_id,
            vec![step("act", agent("act"))],
            PlaybookTrigger::Manual {},
        ))
        .unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("id exceeds")),
            "oversize id must be named: {errs:?}"
        );

        let long_ref = "a".repeat(MAX_TARGET_REF_CHARS + 1);
        let errs = validate(&spec(
            "pb-1",
            vec![step("act", agent(&long_ref))],
            PlaybookTrigger::Manual {},
        ))
        .unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("target agent exceeds")),
            "oversize target ref must be named: {errs:?}"
        );

        let long_match = "x".repeat(MAX_MATCH_TEXT_CHARS + 1);
        let errs = validate(&spec(
            "pb-1",
            vec![step("act", agent("act"))],
            message_trigger(&long_match),
        ))
        .unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("match_text exceeds")),
            "oversize match_text must be named: {errs:?}"
        );
    }

    #[test]
    fn duplicate_names_report_duplicates_without_cycle_noise() {
        let errs = validate(&spec(
            "pb-1",
            vec![step("a", agent("act")), step("a", agent("act"))],
            PlaybookTrigger::Manual {},
        ))
        .unwrap_err();
        assert!(
            errs.iter().any(|e| e == "duplicate step name \"a\""),
            "must report the duplicate name: {errs:?}"
        );
        assert!(
            !errs.iter().any(|e| e.contains("cycle")),
            "duplicate names must not masquerade as a topology error: {errs:?}"
        );
    }

    #[test]
    fn old_brain_target_shape_deserializes_with_route_none() {
        let target: PlaybookTarget =
            serde_json::from_str(r#"{"kind":"brain","capability_id":"c"}"#).unwrap();
        assert_eq!(
            target,
            PlaybookTarget::Brain {
                capability_id: "c".to_string(),
                route: None,
            }
        );
    }

    #[test]
    fn routed_brain_target_round_trips() {
        let target = PlaybookTarget::Brain {
            capability_id: "cap-1".to_string(),
            route: Some(PlaybookRoute {
                kind: PlaybookRouteKind::Team,
                ref_: "auditors".to_string(),
            }),
        };
        let json = serde_json::to_string(&target).unwrap();
        assert!(
            json.contains(r#""ref":"auditors""#),
            "route ref must serialize under the `ref` key: {json}"
        );
        assert_eq!(
            serde_json::from_str::<PlaybookTarget>(&json).unwrap(),
            target
        );
    }
}
