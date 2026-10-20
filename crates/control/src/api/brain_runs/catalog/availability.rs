//! Check explicit profile references without exposing private settings or
//! changing the immutable configuration of executions already accepted.
use super::selection::Entry;
use crate::AppState;
use opencoder_core::{
    fleet::{ExecutionCommand, ExecutionKind, NodeOperation},
    harness::Harness,
    Config,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

pub(super) async fn annotate(
    state: &Arc<AppState>,
    config: &Config,
    entries: &mut [Entry],
) -> anyhow::Result<()> {
    let references =
        opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let kind = entry.value["kind"].as_str()?;
                    if !matches!(kind, "agent" | "operator")
                        || !entry.value["unavailable_reason"].is_null()
                    {
                        return None;
                    }
                    let name = entry.value["target"].as_str()?;
                    let meta = opencoder_core::agent::read_agent_meta(name)?;
                    (meta.harness == Harness::Codex).then_some((
                        index,
                        kind.to_owned(),
                        name.to_owned(),
                        meta.harness_profile?,
                    ))
                })
                .collect::<Vec<_>>()
        });
    if references.is_empty() {
        return Ok(());
    }
    let registered: BTreeSet<String> = state
        .fleet
        .definitions("codex_profile")
        .await?
        .iter()
        .filter_map(|value| value["name"].as_str().map(str::to_owned))
        .collect();
    if references.iter().all(|(_, _, _, p)| registered.contains(p)) {
        return Ok(());
    }
    let nodes = state.hub.views().await;
    let replies = futures::future::join_all(nodes.iter().filter(|n| n.online).map(|node| async move {
        let reply = tokio::time::timeout(
            Duration::from_secs(2),
            state.hub.call(
                &node.registration.id,
                NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: "execution_profiles".into(),
                        input: json!({}),
                    },
                },
            ),
        )
        .await;
        (node, reply.ok().filter(|r| r.status == 200).map(|r| r.body))
    }))
    .await;
    for (index, kind, name, profile) in references {
        if registered.contains(&profile) {
            continue;
        }
        let execution_kind = if kind == "agent" {
            ExecutionKind::Agent
        } else {
            ExecutionKind::Operator
        };
        let mut unknown = false;
        let mut found = false;
        for (node, body) in &replies {
            if !node.registration.kinds.contains(&execution_kind) {
                continue;
            }
            match body.as_ref().and_then(|body| profile_names(&body[&kind])) {
                Some(names) => found |= names.contains(profile.as_str()),
                // Older or unreachable nodes may have a valid private profile.
                // Keep their existing admission behavior instead of guessing.
                None => unknown = true,
            }
        }
        if !found && !unknown {
            entries[index].value["unavailable_reason"] = json!(format!(
                "Codex profile {profile} unavailable for agent {name}"
            ));
        }
    }
    Ok(())
}

fn profile_names(value: &Value) -> Option<BTreeSet<&str>> {
    value.as_array()?.iter().map(Value::as_str).collect()
}
