//! A finite planning/scheduling activation, shared by the mounted CLI and
//! injected provider tests. Runtime decisions never alter an adopted plan.
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, Message, RequestPurpose};

pub async fn activate(
    context: &ActivationContext,
    client: &dyn ChatStream,
    model: &str,
) -> Result<ActivationDecision> {
    let mut decision = ActivationDecision {
        run_id: context.run_id.clone(),
        activation: context.activation,
        control_epoch: context.control_epoch,
        reason: "Dispatch every dependency-ready instance in the immutable plan".into(),
        plan: None,
        dispatch: context.ready.clone(),
    };
    if context.plan.is_some() {
        return Ok(decision);
    }
    ensure!(
        context.phase == RunPhase::Planning,
        "plan missing outside planning phase"
    );
    let request = ChatRequest {
        purpose: RequestPurpose::Planning,
        model: model.into(),
        messages: vec![
            Message::system("brain-contract", PROMPT),
            Message::user("brain-context", serde_json::to_string(context)?),
        ],
        tools: vec![],
        tool_choice: None,
        temperature: Some(0.2),
        max_tokens: Some(16384),
        reasoning_effort: None,
        cache_salt: None,
    };
    let mut events = client.chat_stream(request)?;
    let mut final_text = None;
    while let Some(event) = events.recv().await {
        match event {
            LlmEvent::Completed { text, .. } => {
                final_text = Some(text);
                break;
            }
            LlmEvent::Error(error) => anyhow::bail!("planner provider: {error}"),
            _ => {}
        }
    }
    let raw = final_text.context("planner stream ended without completion")?;
    ensure!(raw.len() <= 1024 * 1024, "planner output exceeds 1 MiB");
    let plan: OntologyPlan = serde_json::from_str(raw.trim())
        .context("planner must return a complete ontology JSON plan")?;
    crate::ontology::validate(&plan)?;
    decision.reason = format!("Initial closed plan for {}", context.objective);
    decision.plan = Some(plan);
    Ok(decision)
}

pub const PROMPT: &str = r#"You are the Brain planner. Produce exactly one complete, executable OntologyPlan JSON object. No markdown, no commentary. Plan once; the runtime cannot replan. Use the supplied capability targets and pinned definitions, with supplied fixed plan versions as references, never silently switch to a fixed plan. Agent act may implement a missing capability. All other actions must use supplied definitions. Explicitly declare every external input and every deliverable, including verification. Expose independent steps for parallel execution. Declare shared resource keys for conflicting writes. Choose no automatic retry for externally visible side effects.
Plan contract:
{"schema_version":1,"title":"short title","objective":"goal","inputs":{"input_name":{"description":"how the user provides this input","schema":{"type":"string"},"required":true}},"steps":[{"id":"step-id","label":"label","purpose":"why needed","capability_id":null,"action":{"kind":"agent","target":"act","prompt":"what to execute","output_mode":"text","max_attempts":1},"inputs":{"name":{"schema":{"type":"string"},"binding":{"source":"input","name":"input_name"},"required":true}},"output":{"type":"string"},"acceptance":"what proves success","depends_on":[],"resources":[]}],"deliverables":{"result":{"description":"the actual deliverable","source":{"source":"output","step":"step-id"},"schema":{"type":"string"}}},"references":[]}
Supported types: string, number, integer, boolean, object (properties, required), array (items), null. Optional semantic tag must match exactly across bindings. Bindings: {source:literal,value}, {source:input,name,path}, {source:output,step,path,collect}, {source:item,path}. Paths are JSON pointers; empty selects the whole value. Output binding implies dependency. Action kinds: agent, dag, todos, team. Copy action.definition from capability definition snapshot. JSON output mode requires a JSON value matching the declared output schema. when:{value:binding,equals:value} skips a false branch. foreach:{items:binding,key:JSON_pointer,allow_empty:true} creates instances from an array with unique stable string/integer keys; bind source:item to each item. Expanded outputs require collect:true for joins. collect excludes skipped instances, waits for the entire finite set to finish, and fails on failed instances. depends_on adds control dependencies. resources:[{key:canonical_global_resource,mode:read|write}]. Deliverables may have expected:true for a final verifier, and their schema must match the source. Missing input is a persistent user request. Use capability_id to bind an action to its entity in the supplied capability library. For phenomenon-driven returns (repair, retest, repair again, then release), declare flow:{entry:action_id,max_visits_per_action:20,transitions:[{from:action_id,to:next_action_id_or_null,label:phenomenon,when:{value:binding,equals:value}}]}. Flow transitions are deterministic: one matching condition wins, one optional unconditional edge is the fallback, and to:null finishes and verifies deliverables. Conditional loops create new durable visits. In flow mode do not use depends_on, when or foreach on steps. Inputs read the latest visit of the named action; required:false allows missing first-visit feedback. Use structured verifier output (passed:boolean,issues:string); route passed:false back to repair and passed:true to release. Never release after failed verification or a visit-limit error. DAG mode remains acyclic; do not reference undeclared actions. Keep the plan small and complete."#;
