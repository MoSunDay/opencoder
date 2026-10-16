use super::{completed, model::parse_after, submit, support::Fleet};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Model {
    consumed: Mutex<Vec<Value>>,
}

impl ChatStream for Model {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let text = request.messages.last().unwrap().text();
        let answer = if request.purpose == RequestPurpose::Planning {
            let context: Value = serde_json::from_str(&text)?;
            let layer = context["run"]["layer"].as_u64().unwrap() as usize;
            let assessments = if layer == 0 {
                json!({})
            } else {
                let id = context["plan"]["layers"][layer - 1]["layer_id"]
                    .as_str()
                    .unwrap();
                let source = context["operations"].as_array().unwrap().last().unwrap();
                let id_exec = source["execution_id"].as_str().unwrap();
                assert_eq!(context["execution_artifacts"][id_exec], json!([]));
                let output: Value =
                    serde_json::from_str(context["summaries"][id_exec].as_str().unwrap()).unwrap();
                if layer == 1 {
                    assert_eq!(output, json!({"seed":"adapter","revision":"team-adapter"}));
                } else {
                    let result: Value =
                        serde_json::from_str(output["verify"].as_str().unwrap()).unwrap();
                    assert_eq!(result["passed"], true);
                    assert_eq!(result["revision"], "team-adapter");
                }
                json!({id:{"met":true,"reason":"complete structured result checked"}})
            };
            if layer == 2 {
                json!({"decision":"complete","reason":"Team to TODO verified",
                    "summary":"all inputs verified","assessments":assessments})
            } else {
                let node = &context["plan"]["nodes"][layer];
                let inputs = if layer == 0 {
                    json!({"seed":{"kind":"root","name":"seed"}})
                } else {
                    let source = &context["operations"][0]["execution_id"];
                    json!({"output":{"kind":"execution","execution_id":source,"path":""},
                        "files":{"kind":"execution","execution_id":source,"source":"artifacts","path":""},
                        "origin":{"kind":"value","value":source}})
                };
                json!({"decision":"dispatch_layer","layer":layer+1,"reason":"transfer current output",
                    "assessments":assessments,"assignments":[{"node_id":node["node_id"],
                    "capability_id":node["capability_id"],"inputs":inputs,"reason":"check exact values"}]})
            }
        } else if text.contains("规划下一轮讨论的核心问题") {
            json!({"question":"check the input","participants":["plan"],"rationale":"inspect"})
        } else if text.contains("判断团队是否已对齐") {
            json!({"summary":"verified input","aligned":true,"ambiguities":[]})
        } else if text.contains("判断话题是否可以收尾") {
            let inputs = parse_after(&text, "Layered inputs:\n")?;
            assert_eq!(inputs["seed"], "adapter");
            json!({"complete":true,"final_summary":json!({"seed":inputs["seed"],
                "revision":"team-adapter"}).to_string()})
        } else if text.contains("你是团队成员") {
            json!({"reviewed":true})
        } else if text.starts_with("Decide the next workflow operation.") {
            let state = parse_after(&text, "STATE=")?;
            if state["todos"]["verify"]["status"] == "passed" {
                json!({"operation":"complete","reason":"verified"})
            } else {
                json!({"operation":"dispatch","todos":[{"todo_id":"verify","context_mode":"new"}],"reason":"ready"})
            }
        } else if text.starts_with("Accept or reject one TODO candidate.") {
            json!({"operation":"accept","reason":"values verified","mark_milestone":false})
        } else if text.starts_with("Complete exactly one focused TODO.") {
            let objective = parse_after(&text, "WORKFLOW_OBJECTIVE=")?;
            let inputs = parse_after(objective.as_str().unwrap(), "Layered inputs:\n")?;
            assert_eq!(inputs["output"]["seed"], "adapter");
            assert_eq!(inputs["output"]["revision"], "team-adapter");
            assert_eq!(inputs["files"], json!([]));
            self.consumed.lock().unwrap().push(inputs.clone());
            let result = json!({"passed":true,"revision":inputs["output"]["revision"],"origin":inputs["origin"]});
            json!({"status":"candidate","summary":"verified","result":result.to_string(),
                "verification":"exact inputs and source checked","evidence_refs":[],
                "recovery_context":{"summary":"verified","refs":[]}})
        } else {
            anyhow::bail!("unexpected adapter request: {text}");
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender.try_send(LlmEvent::Completed {
            text: answer.to_string(),
            tool_calls: vec![],
            usage: None,
        })?;
        Ok(receiver)
    }
}

#[tokio::test]
async fn team_and_todo_transfer_structured_outputs_and_empty_artifact_indexes() {
    let model = Arc::new(Model::default());
    let fleet = Fleet::new(1, model.clone()).await;
    let saved = fleet
        .call(
            "POST",
            "/api/teams",
            json!({"name":"io-team","captain":"act",
        "members":[{"agent":"act"},{"agent":"plan"}]}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    let team = fleet
        .state
        .fleet
        .definition("team", "io-team")
        .await
        .unwrap()
        .unwrap();
    let todo = json!({"schema_version":1,"id":"io-todo","name":"io-todo","objective":"check transferred input",
        "todos":[{"id":"verify","title":"Verify","requirement_background":"IO contract",
        "instructions":"check exact values","agent":"act","max_attempts":1,
        "acceptance":{"criteria":"values and origin match"}}],"metadata":{}});
    for (id, kind, target, definition, inputs, outputs) in [
        (
            "io-team",
            "team",
            "io-team",
            team,
            vec!["seed"],
            vec!["seed", "revision"],
        ),
        (
            "io-todo",
            "todos",
            "io-todo",
            todo,
            vec!["output", "files", "origin"],
            vec!["verify"],
        ),
    ] {
        fleet
            .state
            .fleet
            .put_definition(
                "brain_capability",
                id,
                &json!({"id":id,"kind":kind,
            "target":target,"version":"fixture","summary":id,"input_desc":"bound values",
            "output_desc":"checked result","required_inputs":inputs,"required_outputs":outputs,
            "definition":definition}),
            )
            .await
            .unwrap();
    }
    let plan = json!({"schema_version":7,"title":"Adapter flow","objective":"Team to TODO input transfer",
        "layers":[
            {"layer_id":"team","title":"Team","task":"review input","objective":"report current values","success_criteria":"values exist"},
            {"layer_id":"todo","title":"TODO","task":"check output","objective":"verify source","success_criteria":"candidate checked"}],
        "nodes":[
            {"node_id":"team","layer_id":"team","title":"Team","objective":"review input","capability_id":"io-team"},
            {"node_id":"todo","layer_id":"todo","title":"TODO","objective":"verify transferred values","capability_id":"io-todo"}]});
    submit(&fleet, "brain-io-adapters", "adapter", plan).await;
    let view = completed(&fleet, "brain-io-adapters").await;
    let consumed = model.consumed.lock().unwrap().clone();
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0]["origin"], view["operations"][0]["execution_id"]);
    assert_eq!(view["operations"][1]["execution_kind"], "todos");
    fleet.shutdown().await;
}
