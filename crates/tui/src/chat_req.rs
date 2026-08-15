//! Annotation editor accessor methods for [`ChatView`].
//!
//! Extracted to a sibling module to keep `chat.rs` under its line cap.

use crate::chat::ChatView;

impl ChatView {
    /// Return the editable annotation text: prefer the explicitly saved
    /// `annotation_text`, otherwise fall back to the first user prompt.
    pub fn last_annotation_text(&self) -> Option<String> {
        if let Some(r) = &self.annotation_text {
            if !r.trim().is_empty() {
                return Some(r.clone());
            }
        }
        self.first_prompt.clone()
    }

    /// Save the annotation text verbatim (byte-for-byte, matching the
    /// persisted `sessions.requirement`).
    pub fn update_annotation_text(&mut self, text: &str) {
        self.annotation_text = Some(text.to_string());
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use crate::chat::ChatView;

    #[test]
    fn returns_explicit_annotation_when_set() {
        let mut chat = ChatView::default();
        chat.annotation_text = Some("my task".into());
        assert_eq!(chat.last_annotation_text().as_deref(), Some("my task"));
    }

    #[test]
    fn falls_back_to_first_prompt() {
        let mut chat = ChatView::default();
        chat.first_prompt = Some("first message".into());
        assert_eq!(
            chat.last_annotation_text().as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn returns_none_when_nothing_set() {
        let chat = ChatView::default();
        assert_eq!(chat.last_annotation_text(), None);
    }

    #[test]
    fn empty_annotation_falls_back_to_first_prompt() {
        let mut chat = ChatView::default();
        chat.annotation_text = Some("   ".into());
        chat.first_prompt = Some("real prompt".into());
        assert_eq!(chat.last_annotation_text().as_deref(), Some("real prompt"));
    }

    #[test]
    fn update_annotation_text_preserves_raw_bytes() {
        let mut chat = ChatView::default();
        chat.update_annotation_text("tab\there\r\nbell\u{7}");
        assert_eq!(
            chat.annotation_text.as_deref(),
            Some("tab\there\r\nbell\u{7}")
        );
    }
}
