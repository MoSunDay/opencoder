//! `/model` provider add/edit form: name / model_id / base_url / api_key /
//! headers. Save produces a `ProviderPatch`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;

use super::headers::{HeaderAction, HeadersEditor};
use super::patch::ProviderPatch;
use super::state::{mask_key, ModelMenu, ModelOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderField {
    Name,
    ModelId,
    BaseUrl,
    ApiKey,
    Headers,
    Save,
    Cancel,
}

impl ProviderField {
    const ORDER: [ProviderField; 7] = [
        ProviderField::Name,
        ProviderField::ModelId,
        ProviderField::BaseUrl,
        ProviderField::ApiKey,
        ProviderField::Headers,
        ProviderField::Save,
        ProviderField::Cancel,
    ];
    pub fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }
    pub fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

#[derive(Clone)]
pub struct ProviderForm {
    pub name: String,
    /// `true` when editing an existing provider (Name field is read-only).
    pub name_readonly: bool,
    /// Char-index edit cursor within `name`.
    pub name_cursor: usize,
    pub model_id: String,
    /// Char-index edit cursor within `model_id`.
    pub model_id_cursor: usize,
    pub base_url: String,
    /// Char-index edit cursor within `base_url`.
    pub base_url_cursor: usize,
    pub(crate) api_key_input: String,
    /// Char-index edit cursor within `api_key_input`.
    pub(crate) api_key_cursor: usize,
    pub(crate) api_key_original: String,
    pub(crate) api_key_edited: bool,
    pub headers: HeadersEditor,
    /// When true, keys route to the headers editor instead of form navigation.
    pub headers_active: bool,
    pub focus: ProviderField,
    pub error: Option<String>,
}

impl ProviderForm {
    /// Create a form for editing an existing provider entry.
    pub fn from_existing(
        name: &str,
        base_url: &str,
        model_id: &str,
        api_key: &str,
        headers: Vec<(String, String)>,
    ) -> Self {
        ProviderForm {
            name: name.to_string(),
            name_readonly: true,
            name_cursor: name.chars().count(),
            model_id: model_id.to_string(),
            model_id_cursor: model_id.chars().count(),
            base_url: base_url.to_string(),
            base_url_cursor: base_url.chars().count(),
            api_key_input: String::new(),
            api_key_cursor: 0,
            api_key_original: api_key.to_string(),
            api_key_edited: false,
            headers: HeadersEditor::new(headers),
            headers_active: false,
            focus: ProviderField::ModelId,
            error: None,
        }
    }

    /// Create a blank form for adding a new provider.
    pub fn new_blank(config: &Config) -> Self {
        let base_url = config.base_url_for(config.provider_id());
        ProviderForm {
            name: String::new(),
            name_readonly: false,
            name_cursor: 0,
            model_id: String::new(),
            model_id_cursor: 0,
            base_url: base_url.clone(),
            base_url_cursor: base_url.chars().count(),
            api_key_input: String::new(),
            api_key_cursor: 0,
            api_key_original: String::new(),
            api_key_edited: false,
            headers: HeadersEditor::new(Vec::new()),
            headers_active: false,
            focus: ProviderField::Name,
            error: None,
        }
    }

    /// First-run form seeded from the effective config. Defaults already give
    /// a useful OpenAI-compatible provider/model/base URL, so fresh users land
    /// on the API-key field while every field remains editable.
    pub fn new_onboarding(config: &Config) -> Self {
        let name = config.provider_id().to_string();
        let provider = config.provider_for(&name).unwrap_or(&config.provider);
        let model_id = config.model_id().to_string();
        let base_url = provider.base_url.clone();
        let api_key = provider.api_key.clone().unwrap_or_default();
        ProviderForm {
            name_cursor: name.chars().count(),
            name,
            name_readonly: false,
            model_id_cursor: model_id.chars().count(),
            model_id,
            base_url_cursor: base_url.chars().count(),
            base_url,
            api_key_input: String::new(),
            api_key_cursor: 0,
            api_key_original: api_key,
            api_key_edited: false,
            headers: HeadersEditor::new(
                provider
                    .headers
                    .iter()
                    .map(|h| (h.name.clone(), h.value.clone()))
                    .collect(),
            ),
            headers_active: false,
            focus: ProviderField::ApiKey,
            error: None,
        }
    }

    /// Paste text into the focused field at the cursor (mirrors the `Char`
    /// arm of `handle_key`).
    pub fn paste_into(&mut self, text: &str) {
        if self.headers_active && self.focus == ProviderField::Headers {
            self.headers.paste_into(text);
            return;
        }
        match self.focus {
            ProviderField::Name if !self.name_readonly => {
                let idx = self.name_cursor.min(self.name.chars().count());
                let (s, i) = crate::composer::insert_str(&self.name, idx, text);
                self.name = s;
                self.name_cursor = i;
            }
            ProviderField::ModelId => {
                let idx = self.model_id_cursor.min(self.model_id.chars().count());
                let (s, i) = crate::composer::insert_str(&self.model_id, idx, text);
                self.model_id = s;
                self.model_id_cursor = i;
            }
            ProviderField::BaseUrl => {
                let idx = self.base_url_cursor.min(self.base_url.chars().count());
                let (s, i) = crate::composer::insert_str(&self.base_url, idx, text);
                self.base_url = s;
                self.base_url_cursor = i;
            }
            ProviderField::ApiKey => {
                if !self.api_key_edited {
                    self.api_key_input.clear();
                    self.api_key_edited = true;
                    self.api_key_cursor = 0;
                }
                let idx = self.api_key_cursor.min(self.api_key_input.chars().count());
                let (s, i) = crate::composer::insert_str(&self.api_key_input, idx, text);
                self.api_key_input = s;
                self.api_key_cursor = i;
            }
            _ => {}
        }
    }

    /// Apply `op` to the focused editable text field's (text, cursor).
    /// Read-only name, ApiKey (own edited-flag logic), and non-text fields
    /// are handled by their dedicated arms in `handle_key`.
    fn edit_text<F>(&mut self, op: F)
    where
        F: FnOnce(&mut String, &mut usize),
    {
        match self.focus {
            ProviderField::Name if !self.name_readonly => op(&mut self.name, &mut self.name_cursor),
            ProviderField::ModelId => op(&mut self.model_id, &mut self.model_id_cursor),
            ProviderField::BaseUrl => op(&mut self.base_url, &mut self.base_url_cursor),
            _ => {}
        }
    }

    pub(crate) fn api_key_display(&self) -> String {
        if self.api_key_edited {
            "*".repeat(self.api_key_input.chars().count())
        } else {
            mask_key(&self.api_key_original)
        }
    }

    pub(crate) fn resolve_api_key(&self) -> Option<String> {
        if self.api_key_edited {
            Some(self.api_key_input.clone())
        } else {
            None
        }
    }

    pub fn build_patch(&self) -> ProviderPatch {
        ProviderPatch {
            name: self.name.clone(),
            model_id: self.model_id.clone(),
            base_url: self.base_url.clone(),
            api_key: self.resolve_api_key(),
            headers: self.headers.pairs.clone(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("provider name must not be empty".into());
        }
        if self.name.trim().len() < 2 {
            return Err("provider name must be at least 2 chars (e.g. `bigmodel`)".into());
        }
        if self.model_id.trim().is_empty() {
            return Err("model_id must not be empty".into());
        }
        if self.model_id.trim().len() < 2 {
            return Err("model_id must be at least 2 chars (e.g. `glm-5.2`)".into());
        }
        if self.base_url.trim().is_empty() {
            return Err("base_url must not be empty".into());
        }
        Ok(())
    }
}

/// Handle a key in provider-form mode.
pub fn handle_key(mut form: ProviderForm, k: KeyEvent) -> (ModelOutcome, Option<ModelMenu>) {
    // When headers sub-mode is active, route there first (Ctrl+L/U clear the
    // active header name/value inside HeadersEditor).
    if form.headers_active && form.focus == ProviderField::Headers {
        match form.headers.handle_key(k) {
            HeaderAction::Exit => {
                form.headers_active = false;
            }
            HeaderAction::Active => {}
        }
        return (ModelOutcome::Idle, Some(ModelMenu::Form(form)));
    }

    form.error = None;
    // Ctrl+L / Ctrl+U: clear the focused text field. ApiKey also flips
    // api_key_edited so the cleared buffer is persisted as an edit (same
    // semantics as the Backspace branch). Read-only name is a no-op.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('l')
            | KeyCode::Char('\u{c}')
            | KeyCode::Char('u')
            | KeyCode::Char('\u{15}') => match form.focus {
                ProviderField::ApiKey => {
                    form.api_key_input.clear();
                    form.api_key_cursor = 0;
                    form.api_key_edited = true;
                }
                _ => form.edit_text(|text, cur| {
                    text.clear();
                    *cur = 0;
                }),
            },
            _ => {}
        }
        return (ModelOutcome::Idle, Some(ModelMenu::Form(form)));
    }
    match k.code {
        KeyCode::Esc => return (ModelOutcome::Cancel, None),
        KeyCode::Tab => form.focus = form.focus.next(),
        KeyCode::BackTab => form.focus = form.focus.prev(),
        KeyCode::Up => form.focus = form.focus.prev(),
        KeyCode::Down => form.focus = form.focus.next(),
        KeyCode::Left => form.edit_text(|_, cur| *cur = cur.saturating_sub(1)),
        KeyCode::Right => form.edit_text(|text, cur| *cur = (*cur + 1).min(text.chars().count())),
        KeyCode::Enter => match form.focus {
            ProviderField::Headers => {
                form.headers_active = true;
            }
            ProviderField::Save => {
                if let Err(e) = form.validate() {
                    form.error = Some(e);
                    return (ModelOutcome::Idle, Some(ModelMenu::Form(form)));
                }
                let json = form.build_patch().to_json();
                return (ModelOutcome::Save(json), None);
            }
            ProviderField::Cancel => return (ModelOutcome::Cancel, None),
            _ => form.focus = form.focus.next(),
        },
        KeyCode::Backspace => match form.focus {
            ProviderField::Name | ProviderField::ModelId | ProviderField::BaseUrl => {
                form.edit_text(|text, cur| {
                    let idx = (*cur).min(text.chars().count());
                    if let Some((s, i)) = crate::composer::backspace(text, idx) {
                        *text = s;
                        *cur = i;
                    }
                });
            }
            ProviderField::ApiKey => {
                if !form.api_key_edited {
                    form.api_key_input.clear();
                    form.api_key_edited = true;
                    form.api_key_cursor = 0;
                }
                let idx = form.api_key_cursor.min(form.api_key_input.chars().count());
                if let Some((s, i)) = crate::composer::backspace(&form.api_key_input, idx) {
                    form.api_key_input = s;
                    form.api_key_cursor = i;
                }
            }
            _ => {}
        },
        KeyCode::Char(c) => {
            // Ignore chars meant for headers when not in headers mode.
            match form.focus {
                ProviderField::Name | ProviderField::ModelId | ProviderField::BaseUrl => {
                    form.edit_text(|text, cur| {
                        let idx = (*cur).min(text.chars().count());
                        let (s, i) = crate::composer::insert_char(text, idx, c);
                        *text = s;
                        *cur = i;
                    });
                }
                ProviderField::ApiKey => {
                    if !form.api_key_edited {
                        form.api_key_input.clear();
                        form.api_key_edited = true;
                        form.api_key_cursor = 0;
                    }
                    let idx = form.api_key_cursor.min(form.api_key_input.chars().count());
                    let (s, i) = crate::composer::insert_char(&form.api_key_input, idx, c);
                    form.api_key_input = s;
                    form.api_key_cursor = i;
                }
                _ => {}
            }
        }
        _ => {}
    }
    (ModelOutcome::Idle, Some(ModelMenu::Form(form)))
}

#[cfg(test)]
mod tests {
    //! `validate()` guards the last line of defense before a `ProviderPatch`
    //! is built: it must reject inputs that would produce a malformed `model`
    //! like `m/g`. These tests construct forms directly (validate is private to
    //! this module, reachable from its child `tests` module).
    use super::*;

    fn blank_form() -> ProviderForm {
        ProviderForm::new_blank(&Config::default())
    }

    #[test]
    fn validate_rejects_too_short_name_and_model_id() {
        // `name="m"` + `model_id="g"` would build `m/g`, whose model_id() is a
        // single char and silently breaks every request. validate must stop it.
        let mut form = blank_form();
        form.name = "m".into();
        form.model_id = "g".into();
        form.base_url = "https://api.example.com/v1".into();
        let res = form.validate();
        assert!(res.is_err(), "`m/g` must not validate");
        let msg = res.unwrap_err();
        assert!(
            msg.contains("name") || msg.contains("model"),
            "error should point at the short field; got: {msg}"
        );
    }

    #[test]
    fn validate_accepts_well_formed_provider() {
        let mut form = blank_form();
        form.name = "bigmodel".into();
        form.model_id = "glm-5.2".into();
        form.base_url = "https://open.bigmodel.cn/api/coding/paas/v4".into();
        assert!(
            form.validate().is_ok(),
            "bigmodel/glm-5.2 with a real base_url should validate"
        );
    }
}
