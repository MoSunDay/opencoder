use super::client::ServerClient;
use crate::worker::UiEvent;
use opencoder_session::{
    tools::question::{AskOutcome, QuestionPayload},
    QuestionHub, SessionEvent,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct QuestionBridge {
    pub client: ServerClient,
    pub execution: String,
    pub hub: Arc<QuestionHub>,
    pub ui: mpsc::Sender<UiEvent>,
    pub detach: CancellationToken,
}
impl QuestionBridge {
    /// The existing TUI dialog resolves its hub. Forward that answer into the
    /// server's waiting question without ever running a local agent.
    pub fn start(&self, call_id: String, input: &Value) {
        let Some(prompt) = crate::question_menu::prompt_from_input(&call_id, input) else {
            return;
        };
        if self
            .hub
            .waiting_questions()
            .iter()
            .any(|(id, _)| *id == call_id)
        {
            return;
        }
        let answer = self.hub.ask_with_payload(
            &call_id,
            QuestionPayload {
                question: prompt.question,
                options: prompt.options,
            },
        );
        let bridge = self.clone();
        tokio::spawn(async move {
            let answer = tokio::select! {
                _ = bridge.detach.cancelled() => { bridge.hub.abandon(&call_id); return; }
                answer = async {
                    match answer {
                        AskOutcome::Answered(answer) => Some(answer),
                        AskOutcome::Pending(rx) => rx.await.ok(),
                    }
                } => answer,
            };
            let Some(answer) = answer else {
                return;
            };
            bridge.hub.abandon(&call_id);
            bridge.send_answer(&call_id, &answer).await;
        });
    }

    async fn send_answer(&self, call_id: &str, answer: &str) {
        let mut warned = false;
        loop {
            let result = tokio::select! {
                _ = self.detach.cancelled() => return,
                result = self.client.command(&self.execution,"http",json!({"method":"POST","tail":format!("questions/{call_id}/answer"),"body":{"answer":answer}})) => result,
            };
            match result {
                Ok(_) => return,
                Err(error) => {
                    // A response can be lost after Server has accepted it.
                    if let Ok(result) = self
                        .client
                        .command(
                            &self.execution,
                            "http",
                            json!({"method":"GET","tail":"questions"}),
                        )
                        .await
                    {
                        if result["questions"]
                            .as_array()
                            .is_some_and(|questions| !questions.iter().any(|q| q["id"] == call_id))
                        {
                            return;
                        }
                    }
                    if !warned {
                        let _ = self
                            .ui
                            .send(UiEvent::Session(SessionEvent::Status(format!(
                                "Server question answer failed; retrying: {error}"
                            ))))
                            .await;
                        warned = true;
                    }
                }
            }
            tokio::select! {
                _ = self.detach.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            }
        }
    }

    pub async fn restore(&self) {
        let Ok(result) = self
            .client
            .command(
                &self.execution,
                "http",
                json!({"method":"GET","tail":"questions"}),
            )
            .await
        else {
            return;
        };
        if let Some(questions) = result["questions"].as_array() {
            for question in questions {
                let Some(id) = question["id"].as_str() else {
                    continue;
                };
                self.start(id.into(), question);
                let _ = self
                    .ui
                    .send(UiEvent::Session(SessionEvent::ToolStart {
                        id: id.into(),
                        name: "question".into(),
                        input: question.clone(),
                    }))
                    .await;
            }
        }
    }
}
