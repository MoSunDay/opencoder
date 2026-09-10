//! Capture the exact model payload before sending and each received event.
use super::archive::Archive;
use anyhow::Result;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

pub(super) struct RecordedClient {
    pub inner: Arc<dyn ChatStream>,
    pub archive: Archive,
}
fn event_value(event: &LlmEvent) -> Value {
    match event {
        LlmEvent::ProviderState(state) => json!({"type":"provider_state", "state":state}),
        LlmEvent::TextDelta(text) => json!({"type":"text_delta","text":text}),
        LlmEvent::ReasoningDelta(text) => json!({"type":"reasoning_delta","text":text}),
        LlmEvent::ToolCallStart { index, id, name } => {
            json!({"type":"tool_start","index":index,"id":id,"name":name})
        }
        LlmEvent::ToolCallDelta { index, arguments } => {
            json!({"type":"tool_delta","index":index,"arguments":arguments})
        }
        LlmEvent::Completed {
            text,
            tool_calls,
            usage,
        } => {
            json!({"type":"completed","text":text,"usage":usage,"tool_calls":tool_calls.iter().map(|t|json!({"id":t.id,"name":t.name,"input":t.input})).collect::<Vec<_>>()})
        }
        LlmEvent::Retrying { attempt, max } => {
            json!({"type":"retrying","attempt":attempt,"max":max})
        }
        LlmEvent::Error(error) => json!({"type":"error","error":error}),
    }
}
impl ChatStream for RecordedClient {
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let call = self
            .archive
            .request(&self.inner.request_body(&req)?)
            .inspect_err(|e| self.archive.fail(e))?;
        let mut source = match self.inner.chat_stream(req) {
            Ok(source) => source,
            Err(error) => {
                self.archive
                    .response(call, &json!({"type":"error","error":error.to_string()}))
                    .inspect_err(|e| self.archive.fail(e))?;
                return Err(error);
            }
        };
        let (tx, rx) = mpsc::channel(64);
        let archive = self.archive.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=tx.closed()=>break,
                    item=source.recv()=>{
                        let Some(event)=item else {break};
                        if let Err(error)=archive.response(call,&event_value(&event)) {
                            archive.fail(&error); let _=tx.send(LlmEvent::Error(format!("project persistence: {error:#}"))).await; break;
                        }
                        if tx.send(event).await.is_err(){break;}
                    }
                }
            }
        });
        Ok(rx)
    }
    fn request_body(&self, req: &ChatRequest) -> Result<Value> {
        self.inner.request_body(req)
    }
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }
    fn embed(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        self.inner.embed(texts, model)
    }
}
