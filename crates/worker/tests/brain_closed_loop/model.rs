use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Default)]
pub struct Model {
    pub contexts: Mutex<Vec<Value>>,
}

impl ChatStream for Model {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        anyhow::ensure!(
            request.purpose == RequestPurpose::Planning,
            "native capabilities must execute binaries"
        );
        let context: Value = serde_json::from_str(&request.messages.last().unwrap().text())?;
        for capability in context["capabilities"].as_array().unwrap() {
            assert_eq!(capability["summary"], capability["capability_id"]);
            assert_eq!(capability["required_inputs"], json!(["args"]));
            assert!(!capability["required_outputs"]
                .as_array()
                .unwrap()
                .is_empty());
        }
        self.contexts.lock().unwrap().push(context.clone());
        let decision = decide(&context);
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.try_send(LlmEvent::Completed {
            text: decision.to_string(),
            tool_calls: vec![],
            usage: None,
        })?;
        Ok(rx)
    }
}

fn latest<'a>(context: &'a Value, node: &str) -> Option<&'a Value> {
    context["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|op| op["node_id"] == node)
        .max_by_key(|op| op["activation"].as_u64())
}

fn binding(operation: &Value, path: &str) -> Value {
    json!({"kind":"execution","execution_id":operation["execution_id"],"path":path})
}

fn decide(context: &Value) -> Value {
    let current = context["run"]["layer"].as_u64().unwrap();
    let code = latest(context, "code");
    let test = latest(context, "test");
    let (target, input, met) = match current {
        0 => (1, json!({"kind":"value","value":["1"]}), true),
        1 => {
            let path = if test.is_none() {
                "/code/absent"
            } else {
                "/code/args"
            };
            (2, binding(code.unwrap(), path), true)
        }
        2 => {
            let test = test.unwrap();
            let evidence = &context["summaries"][test["execution_id"].as_str().unwrap()];
            if test["status"] == "error" {
                assert!(
                    evidence.as_str().unwrap().contains("/code/absent"),
                    "{context}"
                );
                (2, binding(code.unwrap(), "/code/args"), false)
            } else {
                let result: Value = serde_json::from_str(evidence.as_str().unwrap()).unwrap();
                let verdict = &result["verify"];
                if verdict["passed"] == true {
                    return json!({"decision":"complete","reason":"the current code revision passes the real test",
                        "summary":"Verified answer() == 2","evidence_execution_ids":[test["execution_id"]],
                        "assessments":{"testing":{"met":true,"reason":verdict["revision"]}}});
                }
                assert_eq!(verdict["passed"], false);
                assert!(verdict["failures"][0]
                    .as_str()
                    .unwrap()
                    .contains("AssertionError"));
                (1, binding(test, "/verify/args"), false)
            }
        }
        _ => unreachable!(),
    };
    let (node, capability) = if target == 1 {
        ("code", "coding")
    } else {
        ("test", "testing")
    };
    let assessments = if current == 0 {
        json!({})
    } else {
        let layer = context["plan"]["layers"][current as usize - 1]["layer_id"]
            .as_str()
            .unwrap();
        json!({layer:{"met":met,"reason":"inspected current execution evidence"}})
    };
    json!({"decision":"dispatch_layer","layer":target,"reason":"choose the capability required by the evidence",
        "reflection":if target <= current {Some("correct the binding or implementation using the failed evidence")} else {None},
        "assessments":assessments,"assignments":[{"node_id":node,"capability_id":capability,
            "inputs":{"args":input},"reason":"explicit source or remediation transfer"}]})
}
