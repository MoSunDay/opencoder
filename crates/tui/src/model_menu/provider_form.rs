//! `/model` provider add/edit form: name / model_id / base_url / api_key /
//! headers. Save produces a `ProviderPatch`.

use crossterm::event::{KeyCode, KeyEvent};
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

pub struct ProviderForm {
    pub name: String,
    /// `true` when editing an existing provider (Name field is read-only).
    pub name_readonly: bool,
    pub model_id: String,
    pub base_url: String,
    pub(crate) api_key_input: String,
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
            model_id: model_id.to_string(),
            base_url: base_url.to_string(),
            api_key_input: String::new(),
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
        ProviderForm {
            name: String::new(),
            name_readonly: false,
            model_id: String::new(),
            base_url: config.base_url_for(config.provider_id()),
            api_key_input: String::new(),
            api_key_original: String::new(),
            api_key_edited: false,
            headers: HeadersEditor::new(Vec::new()),
            headers_active: false,
            focus: ProviderField::Name,
            error: None,
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
    // When headers sub-mode is active, route there first.
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
    match k.code {
        KeyCode::Esc => return (ModelOutcome::Cancel, None),
        KeyCode::Tab => form.focus = form.focus.next(),
        KeyCode::BackTab => form.focus = form.focus.prev(),
        KeyCode::Up => form.focus = form.focus.prev(),
        KeyCode::Down => form.focus = form.focus.next(),
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
            ProviderField::Name if !form.name_readonly => {
                form.name.pop();
            }
            ProviderField::ModelId => {
                form.model_id.pop();
            }
            ProviderField::BaseUrl => {
                form.base_url.pop();
            }
            ProviderField::ApiKey => {
                if !form.api_key_edited {
                    form.api_key_input.clear();
                    form.api_key_edited = true;
                }
                form.api_key_input.pop();
            }
            _ => {}
        },
        KeyCode::Char(c) => {
            // Ignore chars meant for headers when not in headers mode.
            match form.focus {
                ProviderField::Name if !form.name_readonly => form.name.push(c),
                ProviderField::ModelId => form.model_id.push(c),
                ProviderField::BaseUrl => form.base_url.push(c),
                ProviderField::ApiKey => {
                    if !form.api_key_edited {
                        form.api_key_input.clear();
                        form.api_key_edited = true;
                    }
                    form.api_key_input.push(c);
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
