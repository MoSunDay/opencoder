use opencoder_brain::{CapabilityInput, Runtime};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{BrainContentConflict, LibsqlStore, Store};
use std::sync::{Arc, Barrier};

fn input(summary: &str) -> CapabilityInput {
    CapabilityInput {
        capability_type: "tool".into(),
        summary: summary.into(),
        input_desc: "input".into(),
        output_desc: "output".into(),
        eng_inputs: vec!["example".into()],
    }
}
struct PausedEmbed {
    entered: Arc<Barrier>,
    resume: Arc<Barrier>,
}
impl ChatStream for PausedEmbed {
    fn backend(&self) -> &'static str {
        "paused-test"
    }
    fn chat_stream(&self, _: ChatRequest) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        unreachable!()
    }
    fn embed(&self, texts: &[String], model: &str) -> anyhow::Result<Vec<Vec<f32>>> {
        self.entered.wait();
        self.resume.wait();
        MockChatClient::new().embed(texts, model)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn concurrent_edit_during_embedding_is_rejected_at_commit() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let regular = Runtime::new(store.clone(), Arc::new(MockChatClient::new()), "test");
    let original = regular
        .upsert_capability(&input("original"), 1)
        .await
        .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let slow = Runtime::new(
        store,
        Arc::new(PausedEmbed {
            entered: entered.clone(),
            resume: resume.clone(),
        }),
        "test",
    );
    let id = original.capability.id.clone();
    let version = original.content_version();
    let update = tokio::spawn(async move {
        slow.update_capability_if_version(&id, &input("stale"), 2, Some(&version))
            .await
    });
    tokio::task::block_in_place(|| entered.wait());
    let concurrent = regular
        .update_capability(&original.capability.id, &input("concurrent"), 2)
        .await
        .unwrap();
    resume.wait();
    let err = update.await.unwrap().unwrap_err();
    assert!(err.downcast_ref::<BrainContentConflict>().is_some());
    assert_eq!(
        regular
            .get_capability(&original.capability.id)
            .await
            .unwrap()
            .unwrap()
            .content_version(),
        concurrent.content_version()
    );
    let query = opencoder_brain::domain::compose_embed_text(&input("concurrent"));
    assert!(regular.search(&query, 1).await.unwrap()[0].distance.abs() < 1e-6);
}
