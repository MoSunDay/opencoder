//! End-to-end wiring of assistant-output streamlining: the completed assistant
//! text is normalized before it is persisted, while fenced code is preserved
//! verbatim. The live TextDelta stream still carries the original (not asserted
//! here — covered by the unit tests in `src/streamline.rs`).

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, OutputStreamlineConfig, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionState};

fn completed(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.to_string(),
        tool_calls: Vec::new(),
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            ..Default::default()
        }),
    }
}

async fn session_with(
    config: Config,
    client: Arc<dyn ChatStream>,
) -> (tempfile::TempDir, SessionState) {
    let dir = tempfile::tempdir().unwrap();
    let s = SessionState::new(
        "test-session",
        resolve_agent("act").unwrap(),
        config,
        client,
        dir.path().to_path_buf(),
    );
    (dir, s)
}

/// Raw assistant output: prose with trailing whitespace + long blank runs,
/// wrapped around a fenced code block whose interior must be left untouched.
const RAW: &str =
    "line one   \n\n\n\n```rust\nfn x() {   \n\n    let y = 1;   \n}\n```\n\n\n\nfinal   \n";

fn first_assistant_text(s: &SessionState) -> &str {
    let msg = s
        .messages
        .iter()
        .find(|m| m.role == Role::Assistant)
        .expect("an assistant message");
    let block = msg
        .blocks
        .iter()
        .find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("a Text block");
    block
}

#[tokio::test]
async fn persisted_text_is_streamlined_code_preserved() {
    let mock = Arc::new(
        MockChatClient::new().push_script(vec![LlmEvent::TextDelta("x".into()), completed(RAW)]),
    );
    let cfg = Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    };
    let (_dir, mut s) = session_with(cfg, mock as Arc<dyn ChatStream>).await;
    run(&mut s, "do it".into(), |_| {}).await.unwrap();

    let text = first_assistant_text(&s);
    // Blank-run collapsing: no triple (or more) newlines survive in prose.
    assert!(
        !text.contains("\n\n\n"),
        "blank runs should collapse, got: {text:?}"
    );
    // Trailing whitespace stripped from prose lines.
    assert!(!text.contains("one   "), "prose trailing ws stripped");
    assert!(!text.contains("final   "), "prose trailing ws stripped");
    // Fenced-code interior is byte-for-byte intact (incl. its blank line +
    // trailing spaces).
    assert!(
        text.contains("fn x() {   \n\n    let y = 1;   \n"),
        "code fence interior must be verbatim, got: {text:?}"
    );
    // Boundaries between prose and fence collapse to a single blank line.
    assert_eq!(
        text,
        "line one\n\n```rust\nfn x() {   \n\n    let y = 1;   \n}\n```\n\nfinal\n"
    );

    // Sanity: the in-memory transcript (and thus future context) carries the
    // streamlined form, which is strictly shorter than the raw output.
    assert!(text.len() < RAW.len());
}

#[tokio::test]
async fn disabled_keeps_verbatim() {
    let mock = Arc::new(
        MockChatClient::new().push_script(vec![LlmEvent::TextDelta("x".into()), completed(RAW)]),
    );
    let cfg = Config {
        model: "main/glm-5.2".into(),
        output_streamline: OutputStreamlineConfig {
            enabled: false,
            ..Default::default()
        },
        ..Config::default()
    };
    let (_dir, mut s) = session_with(cfg, mock as Arc<dyn ChatStream>).await;
    run(&mut s, "do it".into(), |_| {}).await.unwrap();

    let text = first_assistant_text(&s);
    assert_eq!(text, RAW, "disabled streamlining must be verbatim");
}

// Quiet unused-import guard for `Message` if future edits drop its use.
#[allow(dead_code)]
fn _types(_m: &Message) {}
