use base64::Engine;
use opencoder_core::fleet::MessagePage;
use serde_json::Value;

pub fn message_page_text(detail: &Value) -> String {
    let page: MessagePage = serde_json::from_value(
        detail
            .pointer("/session/messages")
            .cloned()
            .expect("session message page"),
    )
    .expect("valid session message page");
    let mut bytes = Vec::new();
    for chunk in page.chunks {
        assert_eq!(chunk.encoding, "base64");
        bytes.extend(
            base64::engine::general_purpose::STANDARD
                .decode(chunk.bytes_b64)
                .expect("base64 message chunk"),
        );
    }
    String::from_utf8(bytes).expect("message chunks are UTF-8 JSON")
}
