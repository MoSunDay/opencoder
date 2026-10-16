use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

pub struct Model {
    pub contexts: Mutex<Vec<Value>>,
    pub url: Mutex<String>,
    pub gate: Arc<Semaphore>,
    pub load_gate: Arc<Semaphore>,
    pub hold_consumer: bool,
}
impl Model {
    pub fn new(hold_consumer: bool) -> Self {
        Self {
            contexts: Mutex::new(Vec::new()),
            url: Mutex::new(String::new()),
            gate: Arc::new(Semaphore::new(0)),
            load_gate: Arc::new(Semaphore::new(0)),
            hold_consumer,
        }
    }
}

impl ChatStream for Model {
    fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let text = request.messages.last().unwrap().text();
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        if request.purpose == RequestPurpose::Planning {
            let context: Value = serde_json::from_str(&text)?;
            let decision = decision(&context);
            self.contexts.lock().unwrap().push(context.clone());
            if self.hold_consumer
                && context["plan"]["title"] == "IO flow"
                && context["run"]["layer"] == 2
            {
                let gate = self.gate.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = sender.closed() => return,
                        permit = gate.acquire() => permit.unwrap().forget(),
                    }
                    let _ = sender.send(completed(decision.to_string())).await;
                });
            } else {
                sender.try_send(completed(decision.to_string()))?;
            }
        } else if text.contains("Keep the IO source node busy") {
            let gate = self.load_gate.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = sender.closed() => return,
                    permit = gate.acquire() => permit.unwrap().forget(),
                }
                let _ = sender.send(completed("load released".into())).await;
            });
        } else if text.contains("本次运行输入与上游步骤输出（JSON）：") {
            let context = parse_after(&text, "本次运行输入与上游步骤输出（JSON）：")?;
            let payload = &context["inputs"]["payload"];
            assert_eq!(payload["flag"], false);
            assert_eq!(payload["zero"], 0);
            assert_eq!(payload["empty"], json!([]));
            assert!(!context["run_id"].as_str().unwrap().is_empty());
            let output = json!({"marker":payload["seed"],"revision":payload["revision"]});
            sender.try_send(completed(output.to_string()))?;
        } else {
            let inputs = parse_after(&text, "Layered inputs:\n")?;
            if text.contains("Capability: io-producer") {
                let seed = inputs["seed"].as_str().unwrap();
                sender.try_send(completed(
                    json!({"seed":seed,"revision":format!("rev-{seed}"),
                    "flag":false,"zero":0,"empty":[],"items":[["space value"],["second"]]})
                    .to_string(),
                ))?;
            } else {
                let url = self.url.lock().unwrap().clone();
                tokio::spawn(async move {
                    let seed = inputs["seed"].as_str().unwrap();
                    let output = &inputs["output"];
                    if output.get("inspect").is_some() {
                        assert_eq!(output["inspect"]["marker"], seed);
                        assert_eq!(output["inspect"]["origin"], inputs["producer_origin"]);
                        assert_eq!(output["inspect"]["revision"], format!("rev-{seed}"));
                        for (index, item) in output["expand"].as_array().unwrap().iter().enumerate()
                        {
                            assert_eq!(item["namespace"], output["inspect"]["namespace"]);
                            assert_eq!(item["instance"]["index"], index);
                            assert_eq!(
                                item["instance"]["input"],
                                if index == 0 {
                                    json!(["space value"])
                                } else {
                                    json!(["second"])
                                }
                            );
                            assert_eq!(item["readonly"], true);
                        }
                        assert_eq!(output["describe"]["marker"], seed);
                    } else {
                        assert_eq!(output["phase"], "completed");
                    }
                    assert!(inputs["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|file| file["execution"]["id"] == inputs["origin"]));
                    let count = verify_files(&url, &inputs["files"], seed).await;
                    assert_eq!(count, 3);
                    let answer = json!({"passed":true,"revision":format!("rev-{seed}"),"origin":inputs["origin"],"file_count":count});
                    let _ = sender.send(completed(answer.to_string())).await;
                });
            }
        }
        Ok(receiver)
    }
}

fn completed(text: String) -> LlmEvent {
    LlmEvent::Completed {
        text,
        tool_calls: Vec::new(),
        usage: None,
    }
}

pub fn parse_after(text: &str, marker: &str) -> anyhow::Result<Value> {
    let tail = text
        .rsplit_once(marker)
        .ok_or_else(|| anyhow::anyhow!("input marker absent: {text}"))?
        .1;
    serde_json::Deserializer::from_str(tail)
        .into_iter::<Value>()
        .next()
        .context("input value missing")?
        .map_err(Into::into)
}
use anyhow::Context;

fn execution(operation: &Value, path: &str, artifacts: bool) -> Value {
    let mut binding =
        json!({"kind":"execution","execution_id":operation["execution_id"],"path":path});
    if artifacts {
        binding["source"] = json!("artifacts");
    }
    binding
}

fn decision(context: &Value) -> Value {
    let layer = context["run"]["layer"].as_u64().unwrap() as usize;
    let layers = context["plan"]["layers"].as_array().unwrap();
    if context["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|op| op["status"] == "error")
    {
        return json!({"decision":"block","reason":format!("unexpected fixture execution failure: {}",context["summaries"])});
    }
    let assessments = if layer == 0 {
        json!({})
    } else {
        let id = layers[layer - 1]["layer_id"].as_str().unwrap();
        json!({id:{"met":true,"reason":"verified complete current output and artifact references"}})
    };
    if layer == layers.len() {
        let last = context["operations"].as_array().unwrap().last().unwrap();
        let output: Value = serde_json::from_str(
            context["summaries"][last["execution_id"].as_str().unwrap()]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(output["passed"], true);
        return json!({"decision":"complete","reason":"verified data flow","summary":"current values and files verified","assessments":assessments});
    }
    let node = context["plan"]["nodes"][layer].clone();
    let operations = context["operations"].as_array().unwrap();
    let inputs = if layer == 0 {
        json!({"seed":{"kind":"root","name":"seed"}})
    } else if context["plan"]["title"] == "IO flow" && layer == 1 {
        let source = operations.last().unwrap();
        json!({"payload":execution(source,"",false),"items":execution(source,"/items",false),
            "origin":{"kind":"value","value":source["execution_id"]}})
    } else {
        let source = operations.last().unwrap();
        let files = &context["execution_artifacts"][source["execution_id"].as_str().unwrap()];
        assert!(!files.as_array().unwrap().is_empty());
        let mut inputs = json!({"output":execution(source,"",false),"files":execution(source,"",true),
            "origin":{"kind":"value","value":source["execution_id"]},"seed":{"kind":"root","name":"seed"}});
        if context["plan"]["title"] == "IO flow" {
            inputs["producer_origin"] =
                json!({"kind":"value","value":operations[0]["execution_id"]});
        }
        inputs
    };
    json!({"decision":"dispatch_layer","layer":layer+1,"reason":"transfer frozen values",
        "assessments":assessments,"assignments":[{"node_id":node["node_id"],"capability_id":node["capability_id"],"inputs":inputs,"reason":"consume this execution's index"}]})
}

pub async fn verify_files(url: &str, files: &Value, seed: &str) -> usize {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut pending = files.as_array().unwrap().clone();
    let mut count = 0;
    while let Some(file) = pending.pop() {
        if file["step"] != "brain-result" && file["file"] != "payload.txt" {
            continue;
        }
        let mut endpoint = reqwest::Url::parse(&format!(
            "{url}/api/executions/{}/artifact",
            file["execution"]["id"].as_str().unwrap()
        ))
        .unwrap();
        endpoint
            .query_pairs_mut()
            .append_pair("step", file["step"].as_str().unwrap())
            .append_pair("file", file["file"].as_str().unwrap());
        if let Some(index) = file["index"].as_u64() {
            endpoint
                .query_pairs_mut()
                .append_pair("index", &index.to_string());
        }
        let response = client.get(endpoint).send().await.unwrap();
        let status = response.status();
        let bytes = response.bytes().await.unwrap();
        assert!(
            status.is_success(),
            "{file}: {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(bytes.len() as u64, file["bytes"].as_u64().unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            file["sha256"].as_str().unwrap()
        );
        if file["step"] == "brain-result" {
            let manifest: Value = serde_json::from_slice(&bytes).unwrap();
            pending.extend(manifest["artifacts"].as_array().unwrap().clone());
        } else {
            assert_eq!(bytes.as_ref(), format!("{seed}\n").as_bytes());
            count += 1;
        }
    }
    count
}
