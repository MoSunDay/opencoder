//! Requirement editor accessor methods for [`ChatView`].
//!
//! Extracted to a sibling module to keep `chat.rs` under its line cap.

use crate::chat::ChatView;
use crate::terminal_text::sanitize_multiline;

impl ChatView {
    /// Return the editable requirement text: prefer the explicitly saved
    /// `requirement_text`, otherwise fall back to the first user prompt.
    pub fn last_requirement_text(&self) -> Option<String> {
        if let Some(r) = &self.requirement_text {
            if !r.trim().is_empty() {
                return Some(r.clone());
            }
        }
        self.first_prompt.clone()
    }

    /// Save the requirement text (sanitized) for the current session.
    pub fn update_requirement_text(&mut self, text: &str) {
        let text = sanitize_multiline(text);
        self.requirement_text = Some(text.into_owned());
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use crate::chat::ChatView;

    #[test]
    fn returns_explicit_requirement_when_set() {
        let mut chat = ChatView::default();
        chat.requirement_text = Some("my task".into());
        assert_eq!(
            chat.last_requirement_text().as_deref(),
            Some("my task")
        );
    }

    #[test]
    fn falls_back_to_first_prompt() {
        let mut chat = ChatView::default();
        chat.first_prompt = Some("first message".into());
        assert_eq!(
            chat.last_requirement_text().as_deref(),
            Some("first message")
        );
    }

    #[test]
    fn returns_none_when_nothing_set() {
        let chat = ChatView::default();
        assert_eq!(chat.last_requirement_text(), None);
    }

    #[test]
    fn empty_requirement_falls_back_to_first_prompt() {
        let mut chat = ChatView::default();
        chat.requirement_text = Some("   ".into());
        chat.first_prompt = Some("real prompt".into());
        assert_eq!(
            chat.last_requirement_text().as_deref(),
            Some("real prompt")
        );
    }

    #[test]
    fn update_requirement_text_sanitizes() {
        let mut chat = ChatView::default();
        chat.update_requirement_text("hello\r\nworld");
        assert_eq!(
            chat.requirement_text.as_deref(),
            Some("hello\nworld")
        );
    }
}
