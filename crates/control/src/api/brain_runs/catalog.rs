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

pub async fn capabilities(state: &Arc<AppState>) -> anyhow::Result<Vec<Value>> {
    let mut capabilities = vec![
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
        value["required_inputs"] = json!([]);
        value["definition"] = json!({
            "id": value["id"],
            "capability_type": value["capability_type"],
            "summary": value["summary"],
            "input_desc": value["input_desc"],
            "output_desc": value["output_desc"]
        });
        value["version"] = json!("stored");
        value["maturity"] = json!("draft");
        capabilities.push(value);
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
        capabilities.push(json!({"id":format!("agent-{}",agent.name),"kind":"agent","target":agent.name,"summary":agent.description,"input_desc":"Named engineering inputs for this agent","output_desc":"The agent response and execution evidence","required_inputs":[],"definition":{"name":agent.name,"kind":agent.kind,"mode":agent.mode,"prompt":agent.prompt,"tools":agent.tools},"version":"current","maturity":"draft"}));
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
            capabilities.push(json!({"id":format!("todos-{name}-{version}"),"kind":"todos","target":format!("{name}/{version}"),"summary":definition["objective"],"input_desc":"Named inputs for this TODO workflow","output_desc":"Accepted TODO results and evidence","required_inputs":[],"definition":definition,"version":version,"maturity":"draft"}));
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
            .await
            .map_err(|r| anyhow::anyhow!("capability {kind}/{target}: {}", r.body))?;
            capabilities.push(json!({"id":format!("{kind}-{target}"),"kind":kind,"target":target,"summary":definition.get("description").or_else(||definition.get("spec").and_then(|s|s.get("description"))).cloned().unwrap_or(json!("")),"input_desc":format!("Named inputs for this {kind} definition"),"output_desc":format!("The {kind} result and execution evidence"),"required_inputs":[],"definition":snapshot,"version":"current","maturity":"draft"}));
        }
    }
    capabilities.extend(state.fleet.definitions("brain_capability").await?);
    for capability in &mut capabilities {
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
    Ok(capabilities)
}
pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    match capabilities(&state).await {
        Ok(capabilities) => response(RpcReply::ok(json!({"capabilities":capabilities}))),
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

pub async fn record_evidence(
    state: &Arc<AppState>,
    id: &str,
    notice: &opencoder_core::brain::BrainNotice,
) -> anyhow::Result<()> {
    let _process_lock = state.fleet.request_lock("capability", id).await?;
    let _gate = state.brain_gate.lock(&format!("capability:{id}")).await;
    let mut metadata = state
        .fleet
        .definition("brain_capability_meta", id)
        .await?
        .unwrap_or(json!({"maturity":"draft","evidence":[]}));
    let mut evidence = metadata["evidence"].as_array().cloned().unwrap_or_default();
    if !evidence
        .iter()
        .any(|e| e["execution_id"] == notice.execution.id)
    {
        evidence.push(json!({"execution_id":notice.execution.id,"run_id":notice.parent.run_id,"status":notice.status,"at_ms":notice.at_ms}));
        if evidence.len() > 100 {
            evidence.drain(..evidence.len() - 100);
        }
        metadata["evidence"] = json!(evidence);
        state
            .fleet
            .put_definition("brain_capability_meta", id, &metadata)
            .await?;
    }
    Ok(())
}
