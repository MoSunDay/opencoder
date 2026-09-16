//! Real bridge pressure, followed by the same reducer used by the TUI.
use super::*;
use crate::chat::{ChatBlock, ChatView};

fn fold(chat: &mut ChatView, event: UiEvent) {
    match event {
        UiEvent::Session(event) => chat.apply(&event),
        UiEvent::AssistantFinal(text) => chat.reconcile_completed_assistant(&text),
        UiEvent::TurnDone(_) => chat.finalize_assistant(),
    }
}

#[tokio::test]
async fn pressure_cannot_strip_markdown_delimiters_from_an_interim_say() {
    let (tx, mut rx) = mpsc::channel(65);
    // Leave exactly the old shedding threshold free. The barrier confirms
    // that the opening delimiter has been handled before capacity recovers.
    tx.send(UiEvent::Session(SessionEvent::ReasoningDelta(
        "think".into(),
    )))
    .await
    .unwrap();
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);
    forward_event(&pending, SessionEvent::TextDelta("**".into()));
    forward_event(&pending, SessionEvent::Status("barrier".into()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while rx.len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("forwarder must reach the barrier before we release capacity");
    let mut chat = ChatView::default();
    while let Some(event) = rx.recv().await {
        let barrier = matches!(&event,
            UiEvent::Session(SessionEvent::Status(text)) if text == "barrier");
        fold(&mut chat, event);
        if barrier {
            break;
        }
    }
    forward_event(&pending, SessionEvent::TextDelta("中间回复**".into()));
    forward_event(&pending, SessionEvent::LlmRoundEnd);
    forward_event(&pending, SessionEvent::ReasoningDelta("next".into()));
    forward_event(&pending, SessionEvent::TextDelta("**最后回复**".into()));
    forward_event(&pending, SessionEvent::LlmRoundEnd);
    forward_event(&pending, SessionEvent::Done);
    pending
        .send(UiEvent::AssistantFinal("**最后回复**".into()))
        .unwrap();
    pending.send(UiEvent::TurnDone("act".into())).unwrap();
    drop(pending);
    while let Some(event) = rx.recv().await {
        fold(&mut chat, event);
    }
    forwarder.await.unwrap();

    let says: Vec<_> = chat
        .blocks
        .iter()
        .filter_map(|block| match block {
            ChatBlock::Assistant { raw, done, .. } => {
                assert!(done);
                Some(raw.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(says, ["**中间回复**", "**最后回复**"]);
    let text: String = chat
        .flatten()
        .iter()
        .flat_map(|l| &l.spans)
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        !text.contains("**"),
        "completed output still contains raw markup: {text}"
    );
}

#[tokio::test]
async fn say_markdown_survives_pressure_at_every_character_and_terminal_boundary() {
    let source = "## 结论 **bold** 和 `code`\r\n\r\n- *斜体*\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| x | y |";
    for capacity in [1, 2, 65, UI_EVENT_CAPACITY] {
        for terminal in [
            SessionEvent::LlmRoundEnd,
            SessionEvent::Done,
            SessionEvent::Error("cancelled".into()),
        ] {
            let (tx, mut rx) = mpsc::channel(capacity);
            let (pending, forwarder) = spawn_ui_event_forwarder(tx);
            let mut expected = ChatView::default();
            let mut emit = |event: SessionEvent| {
                expected.apply(&event);
                forward_event(&pending, event);
            };
            emit(SessionEvent::LlmRoundStart { started_at_ms: 1 });
            emit(SessionEvent::ReasoningDelta("think".into()));
            for ch in source.chars() {
                emit(SessionEvent::TextDelta(ch.to_string()));
            }
            emit(terminal);
            drop(pending);
            let mut actual = ChatView::default();
            while let Some(event) = rx.recv().await {
                fold(&mut actual, event);
            }
            forwarder.await.unwrap();
            assert_eq!(actual.flatten(), expected.flatten(), "capacity {capacity}");
            assert!(actual.blocks.iter().any(|b| matches!(b,
                ChatBlock::Assistant { raw, done: true, .. }
                    if raw == crate::terminal_text::sanitize_multiline(source).as_ref())));
        }
    }
}

#[tokio::test]
async fn say_retry_and_interleaved_reasoning_are_ordering_barriers() {
    let (tx, mut rx) = mpsc::channel(1);
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);
    let events = [
        SessionEvent::LlmRoundStart { started_at_ms: 1 },
        SessionEvent::ReasoningDelta("failed attempt".into()),
        SessionEvent::TextDelta("**discarded".into()),
        SessionEvent::LlmAttemptReset,
        SessionEvent::ReasoningDelta("retry".into()),
        SessionEvent::TextDelta("**first".into()),
        SessionEvent::TextDelta(" say**".into()),
        SessionEvent::ReasoningDelta("next step".into()),
        SessionEvent::TextDelta("`second".into()),
        SessionEvent::TextDelta(" say`".into()),
        SessionEvent::LlmRoundEnd,
        SessionEvent::Done,
    ];
    let mut expected = ChatView::default();
    for event in events {
        expected.apply(&event);
        forward_event(&pending, event);
    }
    drop(pending);
    let mut actual = ChatView::default();
    while let Some(event) = rx.recv().await {
        fold(&mut actual, event);
    }
    forwarder.await.unwrap();
    assert_eq!(actual.flatten(), expected.flatten());
}

#[tokio::test]
async fn pressure_coalesces_backlog_without_crossing_child_or_reset_events() {
    let (tx, mut rx) = mpsc::channel(1);
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);
    for _ in 0..200 {
        forward_event(&pending, SessionEvent::TextDelta("中*\n".into()));
    }
    forward_event(
        &pending,
        SessionEvent::SubagentChild {
            id: "child".into(),
            ev: Box::new(SessionEvent::TextDelta("child **text**".into())),
        },
    );
    forward_event(&pending, SessionEvent::TranscriptReset(Vec::new()));
    forward_event(&pending, SessionEvent::TextDelta("new transcript".into()));
    drop(pending);
    let mut text = String::new();
    let mut batches = 0;
    loop {
        match rx.recv().await.unwrap() {
            UiEvent::Session(SessionEvent::TextDelta(chunk)) => {
                text.push_str(&chunk);
                batches += 1;
            }
            UiEvent::Session(SessionEvent::SubagentChild { id, ev }) => {
                assert_eq!(id, "child");
                assert!(matches!(*ev, SessionEvent::TextDelta(t) if t == "child **text**"));
                break;
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert_eq!(text, "中*\n".repeat(200));
    assert!(
        batches < 20,
        "a backlogged stream should be batched: {batches}"
    );
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::TranscriptReset(_)))
    ));
    assert!(
        matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::TextDelta(t))) if t == "new transcript")
    );
    assert!(rx.recv().await.is_none());
    forwarder.await.unwrap();
}
