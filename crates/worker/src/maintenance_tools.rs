use crate::{state::Inner, Worker};
use anyhow::Result;
use opencoder_core::{fleet::*, Tool, ToolContext, ToolOutput};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
struct MaintenanceTool {
    worker: Weak<Inner>,
}
#[async_trait::async_trait]
impl Tool for MaintenanceTool {
    fn name(&self) -> &str {
        "node_maintenance"
    }
    fn description(&self) -> &str {
        "Query this node's status, executions, resources, config, models or skills. Inspect, events and control require input.execution with the exact id and kind returned by executions. Only use configure or control when the user's request explicitly authorizes that change; never autonomously repair, delete authentication data or rotate credentials."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "action":{"type":"string","enum":["status","executions","resources","config","models","skills","inspect","events","control","configure"]},
            "input":{"type":"object","description":"inspect/events: {execution:{id,kind},after?}; control: {execution:{id,kind},command:{action,input?}}; configure: config patch","properties":{
                "execution":{"type":"object","properties":{
                    "id":{"type":"string"},
                    "kind":{"type":"string","enum":["agent","dag","team","todos","project","maintenance","system"]}
                },"required":["id","kind"],"additionalProperties":false},
                "after":{"type":"integer"},
                "command":{"type":"object","properties":{"action":{"type":"string"},"input":{}},"required":["action"]}
            }}
        },"required":["action"],"allOf":[
            {"if":{"properties":{"action":{"enum":["inspect","events","control"]}},"required":["action"]},"then":{"required":["input"],"properties":{"input":{"required":["execution"]}}}},
            {"if":{"properties":{"action":{"const":"control"}},"required":["action"]},"then":{"properties":{"input":{"required":["command"]}}}}
        ]})
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let worker = Worker {
            inner: self
                .worker
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("node stopped"))?,
        };
        let command: ExecutionCommand = serde_json::from_value(input)?;
        if !matches!(
            command.action.as_str(),
            "status"
                | "executions"
                | "resources"
                | "config"
                | "models"
                | "skills"
                | "inspect"
                | "events"
                | "control"
                | "configure"
        ) {
            return Ok(ToolOutput::err("unsupported maintenance action"));
        }
        let reply = worker.handle(NodeOperation::Maintenance { command }).await;
        Ok(if reply.status < 300 {
            ToolOutput::ok(serde_json::to_string(&reply.body)?)
        } else {
            ToolOutput::err(serde_json::to_string(&reply.body)?)
        })
    }
}
pub(crate) fn install(worker: &Worker, id: &str) {
    worker
        .inner
        .maintenance
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id.into())
        .or_insert_with(|| {
            opencoder_session::extensions::register(
                id,
                vec![Arc::new(MaintenanceTool {
                    worker: Arc::downgrade(&worker.inner),
                })],
            )
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_schema_requires_typed_execution_reference() {
        let tool = MaintenanceTool {
            worker: Weak::new(),
        };
        let schema = tool.parameters();
        let execution = &schema["properties"]["input"]["properties"]["execution"];
        assert_eq!(execution["required"], json!(["id", "kind"]));
        assert!(execution["properties"]["kind"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("maintenance")));
        assert_eq!(schema["allOf"][0]["then"]["required"], json!(["input"]));
        assert!(tool.description().contains("input.execution"));
    }

    #[tokio::test]
    async fn llm_visible_typed_shape_reaches_all_reference_actions() {
        let dir = tempfile::tempdir().unwrap();
        let worker = Worker::open(
            crate::WorkerOptions {
                name: "maintenance-schema".into(),
                workdir: dir.path().join("work"),
                data_dir: dir.path().join("node"),
                workflow_root: None,
                max_runs: Some(1),
                dag: true,
            },
            Some(Arc::new(opencoder_llm::MockChatClient::new())),
        )
        .await
        .unwrap();
        let execution = json!({"id":"maintenance-missing","kind":"maintenance"});
        for (action, input) in [
            ("inspect", json!({"execution":execution.clone()})),
            ("events", json!({"execution":execution.clone(),"after":0})),
            (
                "control",
                json!({"execution":execution,"command":{"action":"cancel"}}),
            ),
        ] {
            let reply = worker
                .handle(NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: action.into(),
                        input,
                    },
                })
                .await;
            assert_eq!(reply.status, 404, "{action}: {reply:?}");
        }
        worker.shutdown().await.unwrap();
    }
}
