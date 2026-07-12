//! Cheap token estimator. Matches opencoder's heuristic (`length / 4`) closely
//! enough to drive compaction thresholds without calling the model — so
//! compaction can fire on the very first round before any `usage` is reported.

use opencoder_core::Message;

const CHARS_PER_TOKEN: usize = 4;
const PER_MESSAGE_OVERHEAD: usize = 4;

/// Estimate tokens in a raw string (≈ chars/4, minimum 1 for non-empty).
pub fn estimate(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count();
    chars.div_ceil(CHARS_PER_TOKEN)
}

/// Estimate tokens for a slice of messages: sum of each message's full
/// content (text + reasoning + tool-use input + tool-result content) plus a
/// small structural overhead per message.
pub fn estimate_messages(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|m| estimate(&m.estimate_chars()) + PER_MESSAGE_OVERHEAD)
        .sum()
}

/// Estimate tokens for the full session transcript that will be sent to the
/// model (system prompt text + all messages).
pub fn estimate_transcript(system: &str, messages: &[Message]) -> usize {
    estimate(system) + estimate_messages(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate(""), 0);
    }

    #[test]
    fn roughly_chars_divided_by_four_rounding_up() {
        // 8 chars → 2 tokens
        assert_eq!(estimate("abcdefgh"), 2);
        // 9 chars → 3 tokens (ceil)
        assert_eq!(estimate("abcdefghi"), 3);
        // 1 char → 1 token
        assert_eq!(estimate("a"), 1);
    }

    #[test]
    fn non_ascii_counts_codepoints() {
        // 4 hanzi → 1 token (length-based heuristic, not BPE — deliberately cheap)
        assert_eq!(estimate("你好世界"), 1);
    }

    #[test]
    fn estimate_messages_grows_with_count() {
        let m1 = Message::user("id1", "abcdefgh"); // "abcdefgh\n" 9ch → 3 + 4 = 7
        let m2 = Message::assistant("id2"); // 0 + 4 = 4
        let total = estimate_messages(&[m1, m2]);
        assert_eq!(total, 11);
    }

    #[test]
    fn estimate_transcript_combines_system_and_messages() {
        let sys = "abcdefgh"; // 2 tokens
        let m1 = Message::user("id1", "abcdefgh"); // 3 + 4 = 7
        let total = estimate_transcript(sys, &[m1]);
        assert_eq!(total, 9); // 2 + 7
    }

    #[test]
    fn estimate_transcript_empty_both_is_zero() {
        assert_eq!(estimate_transcript("", &[]), 0);
    }

    #[test]
    fn estimate_messages_counts_tool_results_and_tool_use() {
        use opencoder_core::ContentBlock;
        use serde_json::json;

        let mut m = Message::assistant("id1");
        m.blocks.push(ContentBlock::ToolUse {
            id: "call_1".into(),
            name: "bash".into(),
            input: json!({"cmd": "ls -la"}),
        });
        m.blocks.push(ContentBlock::ToolResult {
            tool_use_id: "call_1".into(),
            content: "x".repeat(400),
            is_error: false,
        });
        let total = estimate_messages(&[m]);
        assert!(
            total > 100,
            "tool result (400ch) must be counted, got {total}"
        );
    }
}
