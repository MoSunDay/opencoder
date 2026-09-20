use crate::support::Fleet;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use serde_json::{json, Value};

pub struct BrowserClient;
impl ChatStream for BrowserClient {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let last = request.messages.last().unwrap().text();
        let context = serde_json::from_str::<Value>(&last).ok();
        let value = if let Some(context) =
            context.filter(|c| c.get("run_id").is_some() && c.get("operations").is_some())
        {
            let round = context["round"].as_u64().unwrap();
            let kinds = match round {
                0 => vec!["agent", "team", "dag"],
                1 => vec!["todos", "operator"],
                _ => vec![],
            };
            let evidence: Vec<_> = context["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| op["execution_id"].clone())
                .collect();
            if kinds.is_empty() {
                json!({"decision":"complete","reason":"All five execution types supplied evidence","evidence_execution_ids":evidence})
            } else {
                json!({"decision":"dispatch","capabilities":kinds.iter().map(|kind| json!({"capability_id":format!("browser-{kind}"),"inputs":{}})).collect::<Vec<_>>(),"reason":format!("Browser scheduler round {}", round + 1),"evidence_execution_ids":evidence})
            }
        } else if last.contains("Accept or reject one TODO candidate.") {
            json!({"operation":"accept","reason":"candidate verified","mark_milestone":true})
        } else if last.contains("Decide the next workflow operation.") {
            let runnable = last
                .lines()
                .find_map(|line| line.strip_prefix("RUNNABLE="))
                .unwrap();
            let ids: Vec<String> = serde_json::from_str(runnable)?;
            if ids.is_empty() {
                json!({"operation":"complete","reason":"all passed"})
            } else {
                json!({"operation":"dispatch","todos":ids.iter().map(|id| json!({"todo_id":id,"context_mode":"new"})).collect::<Vec<_>>(),"reason":"ready"})
            }
        } else {
            json!({"question":"inspect","participants":["plan"],"rationale":"review","summary":"verified browser execution","aligned":true,"complete":true,"final_summary":"verified browser execution","status":"candidate","result":"verified browser execution","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}})
        };
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(LlmEvent::Completed {
            text: value.to_string(),
            tool_calls: vec![],
            usage: None,
        })?;
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "mock"
    }
}

pub async fn seed(fleet: &Fleet) {
    let definitions = [
        ("agent", "act", json!({"name":"act"})),
        ("operator", "act", json!({"name":"act"})),
        (
            "team",
            "browser-team",
            json!({"name":"browser-team","captain":"act","members":[{"agent":"act"},{"agent":"plan"}]}),
        ),
        (
            "dag",
            "browser-dag",
            json!({"name":"browser-dag","steps":[{"name":"review","kind":{"type":"agent","prompt":"Review browser evidence"}}]}),
        ),
        (
            "todos",
            "browser-todos/1",
            json!({"schema_version":1,"id":"wf-browser","name":"browser-todos","objective":"finish item","constraints":[],"todos":[{"id":"t1","title":"review","requirement_background":"browser test","instructions":"return candidate","depends_on":[],"agent":"act","max_attempts":2,"acceptance":{"criteria":"candidate exists"}}]}),
        ),
    ];
    for (kind, target, definition) in definitions {
        let id = format!("browser-{kind}");
        fleet.state.fleet.put_definition("brain_capability", &id, &json!({
            "id":id,"kind":kind,"target":target,"summary":format!("浏览器验收 {kind}"),
            "input_desc":"Objective only","output_desc":"Execution evidence","definition":definition,"version":"1"
        })).await.unwrap();
    }
}
