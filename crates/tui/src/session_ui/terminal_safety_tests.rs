use super::replay::replay_messages;
use crate::chat::ChatBlock;
use opencoder_core::{ContentBlock, Message, MessageUsage, Role};

#[test]
fn replay_sanitizes_old_persisted_text_without_mutating_the_message() {
    let dirty = "old\r\x1b[2J\x08\u{009b}new";
    let message = Message {
        id: "assistant".into(),
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text { text: dirty.into() },
            ContentBlock::Reasoning { text: dirty.into() },
        ],
        model: None,
        agent: None,
        usage: MessageUsage::default(),
        created_at: 0,
        synthetic: false,
    };

    let chat = replay_messages("act", std::slice::from_ref(&message));
    for block in &chat.blocks {
        match block {
            ChatBlock::Assistant { raw, .. } | ChatBlock::Thinking { text: raw, .. } => {
                assert!(
                    raw.chars().all(|ch| !ch.is_control() || ch == '\n'),
                    "persisted control reached replayed UI state: {raw:?}"
                );
            }
            _ => {}
        }
    }

    assert_eq!(message.blocks[0].as_text(), Some(dirty));
}
