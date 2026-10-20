use crate::{
    api::{error_400, error_500, response},
    AppState,
};
use axum::{
    extract::{Path, State},
    response::Response,
    Json,
};
use opencoder_core::fleet::RpcReply;
use serde_json::{json, Value};
use std::sync::Arc;

mod availability;
#[path = "catalog/dedup.rs"]
mod dedup;
mod selection;
use selection::{Entry, Origin};

pub struct Library {
    pub capabilities: Vec<Value>,
    pub selection_ids: Vec<String>,
}

pub async fn capabilities(state: &Arc<AppState>) -> anyhow::Result<Vec<Value>> {
    Ok(raw_load(state).await?.capabilities)
}

async fn raw_load(state: &Arc<AppState>) -> anyhow::Result<Library> {
    let builtins = vec![
        json!({
            "id":"builtin-agent-act",
            "kind":"agent",
            "target":"act",
            "summary":"General purpose agent",
            "input_desc":"Named engineering inputs for an agent task",
            "output_desc":"A bounded task result with evidence",
            "required_inputs":[],
            "definition":{"name":"act","kind":"builtin-agent","target":"act"},
            "version":"builtin",
            "maturity":"stable"
        }),
        json!({
            "id":"builtin-operator",
            "kind":"operator",
            "target":"act",
            "summary":"Execute an explicit host operation using the registered Operator",
            "input_desc":"Named inputs describing the host operation",
            "output_desc":"The operation result and execution evidence",
            "required_inputs":[],
            "definition":{"name":"act","kind":"builtin-operator","target":"act"},
            "version":"builtin",
            "maturity":"stable"
        }),
    ];
    let mut capabilities: Vec<_> = builtins
        .into_iter()
        .map(|value| Entry::new(Origin::Builtin, value))
        .collect();
    for capability in state.store.list_brain_capabilities().await? {
        let Some(target) = state
            .fleet
            .definition("capability_target", &capability.capability.id)
            .await?
        else {
            continue;
        };
        let mut value = json!(capability.capability);
        value["kind"] = target["kind"].clone();
        value["target"] = target["target"].clone();
        value["required_inputs"] = target.get("required_inputs").cloned().unwrap_or(json!([]));
        value["required_outputs"] = target.get("required_outputs").cloned().unwrap_or(json!([]));
        match registered_definition(state, &target).await {
            Ok(definition) => value["definition"] = definition,
            Err(error) => {
                value["definition"] = Value::Null;
                value["unavailable_reason"] = json!(error.to_string());
            }
        }
        value["version"] = json!("stored");
        value["maturity"] = json!("draft");
        capabilities.push(Entry::new(Origin::Stored, value));
    }
    let config = opencoder_core::Config::load(&state.workdir)?;
    let agents =
        opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            opencoder_core::agent::list_agents()
                .into_iter()
                .filter_map(|name| opencoder_core::resolve_agent(&name))
                .collect::<Vec<_>>()
        });
    for agent in agents {
        capabilities.push(Entry::new(Origin::Generated, json!({"id":format!("agent-{}",agent.name),"kind":"agent","target":agent.name,"summary":agent.description,"input_desc":"Named engineering inputs for this agent","output_desc":"The agent response and execution evidence","required_inputs":[],"definition":{"name":agent.name,"kind":agent.kind,"mode":agent.mode,"prompt":agent.prompt,"tools":agent.tools},"version":"current","maturity":"draft"})));
    }
    let (_, share) = crate::api_todo_util::share_root(&state.workdir).await?;
    for name in opencoder_core::list_child_dirs(&share.join("todo")) {
        let root = opencoder_core::todo_dir(&share, &name)?;
        for version in opencoder_core::list_child_dirs(&root) {
            if !opencoder_core::todo_context_path(&share, &name, &version)?.is_file()
                && !opencoder_core::todo_version_dir(&share, &name, &version)?
                    .join("workflow.json")
                    .is_file()
            {
                continue;
            }
            let definition = crate::api::template::snapshot(&share, &name, &version)?;
            capabilities.push(Entry::new(Origin::Generated, json!({"id":format!("todos-{name}-{version}"),"kind":"todos","target":format!("{name}/{version}"),"summary":definition["objective"],"input_desc":"Named inputs for this TODO workflow","output_desc":"Accepted TODO results and evidence","required_inputs":[],"definition":definition,"version":version,"maturity":"draft"})));
        }
    }
    for kind in ["dag", "team"] {
        for definition in state.fleet.definitions(kind).await? {
            let Some(target) = definition["name"].as_str().or(definition["id"].as_str()) else {
                continue;
            };
            if kind == "team" && target == "system" {
                continue;
            }
            let snapshot = crate::api::catalog::resolve(
                state,
                &opencoder_core::fleet::CreateExecution {
                    id: format!("{kind}-catalog"),
                    kind: serde_json::from_value(json!(kind))?,
                    target: Some(target.into()),
                    input: json!({}),
                    node_id: None,
                },
            )
            .await;
            let mut capability = json!({"id":format!("{kind}-{target}"),"kind":kind,"target":target,"summary":definition.get("description").or_else(||definition.get("spec").and_then(|s|s.get("description"))).cloned().unwrap_or(json!("")),"input_desc":format!("Named inputs for this {kind} definition"),"output_desc":format!("The {kind} result and execution evidence"),"required_inputs":[],"definition":null,"version":"current","maturity":"draft"});
            match snapshot {
                Ok(Some(snapshot)) => capability["definition"] = snapshot,
                Ok(None) => capability["unavailable_reason"] = json!("definition missing"),
                Err(reply) => capability["unavailable_reason"] = reply.body,
            }
            capabilities.push(Entry::new(Origin::Generated, capability));
        }
    }
    capabilities.extend(
        state
            .fleet
            .definitions("brain_capability")
            .await?
            .into_iter()
            .map(|value| Entry::new(Origin::Stored, value)),
    );
    capabilities.extend(
        super::plan_capabilities::list(state)
            .await?
            .into_iter()
            .map(|value| Entry::new(Origin::Plan, value)),
    );
    for entry in &mut capabilities {
        let capability = &mut entry.value;
        if let Some(meta) = state
            .fleet
            .definition(
                "brain_capability_meta",
                capability["id"].as_str().unwrap_or(""),
            )
            .await?
        {
            capability["maturity"] = meta["maturity"].clone();
            capability["evidence"] = meta["evidence"].clone();
        }
    }
    availability::annotate(state, &config, &mut capabilities).await?;
    Ok(Library {
        selection_ids: selection::ids(&capabilities),
        capabilities: capabilities.into_iter().map(|entry| entry.value).collect(),
    })
}

pub async fn load(state: &Arc<AppState>) -> anyhow::Result<Library> {
    let Library {
        capabilities,
        selection_ids: initial_ids,
    } = raw_load(state).await?;
    let config = opencoder_core::Config::load(&state.workdir)?;
    let (mut capabilities, selected) = dedup::select(config.agent.agents_dir, capabilities).await?;
    let selection_ids = selected
        .iter()
        .filter_map(|value| {
            value["id"]
                .as_str()
                .filter(|id| initial_ids.iter().any(|initial| initial == id))
                .map(str::to_owned)
        })
        .collect();
    for value in &mut capabilities {
        value
            .as_object_mut()
            .unwrap()
            .remove("_catalog_agent_identity");
        if let Some(selected) = selected.iter().find(|v| v["id"] == value["id"]) {
            *value = selected.clone();
        }
    }
    Ok(Library {
        capabilities,
        selection_ids,
    })
}

async fn registered_definition(state: &Arc<AppState>, target: &Value) -> anyhow::Result<Value> {
    let target: opencoder_core::fleet::CapabilityTarget = serde_json::from_value(target.clone())?;
    let request = opencoder_core::fleet::CreateExecution {
        id: format!("{}-catalog", target.kind.prefix()),
        kind: target.kind,
        target: Some(target.target.clone()),
        input: json!({}),
        node_id: None,
    };
    if let Some(definition) = crate::api::catalog::resolve(state, &request)
        .await
        .map_err(|reply| anyhow::anyhow!("capability target {}: {}", target.target, reply.body))?
    {
        return Ok(definition);
    }
    let config = opencoder_core::Config::load(&state.workdir)?;
    let agent = opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir, || {
        opencoder_core::resolve_agent(&target.target)
    })
    .ok_or_else(|| anyhow::anyhow!("capability target {} unavailable", target.target))?;
    Ok(
        json!({"name":agent.name,"kind":agent.kind,"mode":agent.mode,"prompt":agent.prompt,"tools":agent.tools}),
    )
}

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match load(&state).await {
        Ok(library) => response(RpcReply::ok(
            json!({"capabilities":library.capabilities,"selection_ids":library.selection_ids}),
        )),
        Err(e) => error_500(e.to_string()),
    }
}
pub async fn stable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if body["maturity"] != "stable" && body["maturity"] != "draft" {
        return error_400("maturity must be draft or stable".into());
    }
    let _process_lock = match state.fleet.request_lock("capability", &id).await {
        Ok(lock) => lock,
        Err(error) => return error_500(error.to_string()),
    };
    let _gate = state.brain_gate.lock(&format!("capability:{id}")).await;
    let mut metadata = match state.fleet.definition("brain_capability_meta", &id).await {
        Ok(Some(value)) => value,
        Ok(None) => json!({"evidence":[]}),
        Err(error) => return error_500(error.to_string()),
    };
    metadata["maturity"] = body["maturity"].clone();
    match state
        .fleet
        .put_definition("brain_capability_meta", &id, &metadata)
        .await
    {
        Ok(()) => response(RpcReply::ok(metadata)),
        Err(e) => error_500(e.to_string()),
    }
}
