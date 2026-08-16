//! `/skill` toggle list: toggle (←/→), close (Enter/Esc). No form, no
//! delete — skill content lives on disk (`~/.opencoder/skills`); this modal
//! only flips the default-injection toggle persisted in config.

use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::Config;
use serde_json::{Value, json};

use super::state::{SkillMenu, SkillOutcome};

/// A discovered skill with its resolved default-injection state.
#[derive(Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

/// Toggle list over the discovered skills (name-sorted by `discover_skills`).
pub struct SkillList {
    pub entries: Vec<SkillEntry>,
    pub selected: usize,
}

impl SkillList {
    /// Pure constructor: discovered skills merged with the config's per-skill
    /// toggles. A skill missing from config defaults OFF.
    pub fn from_discovered(skills: &[opencoder_core::Skill], config: &Config) -> Self {
        let entries = skills
            .iter()
            .map(|s| SkillEntry {
                name: s.name.clone(),
                description: s.description.clone(),
                enabled: config.skills.get(&s.name).is_some_and(|c| c.enabled),
            })
            .collect();
        Self { entries, selected: 0 }
    }

    /// Discover `~/.opencoder/skills` and build the toggle list.
    pub fn new(config: &Config) -> Self {
        Self::from_discovered(&opencoder_core::discover_skills(), config)
    }

    pub fn selected_entry(&self) -> Option<&SkillEntry> { self.entries.get(self.selected) }

    fn move_up(&mut self) {
        let n = self.entries.len();
        if n > 0 { self.selected = (self.selected + n - 1) % n; }
    }

    fn move_down(&mut self) {
        let n = self.entries.len();
        if n > 0 { self.selected = (self.selected + 1) % n; }
    }
}

/// JSON merge-patch for one toggle flip: `{"skills":{<name>:{"enabled":<bool>}}}`.
pub fn toggle_skill_json(name: &str, enabled: bool) -> Value {
    json!({ "skills": { name: { "enabled": enabled } } })
}

/// One keystroke, mirroring `mcp_menu::list::handle_key`: ←/→ flip the selected
/// entry and return `(Save(json), Some(List))` so the modal stays open; Enter/Esc close.
pub fn handle_key(mut list: SkillList, k: KeyEvent) -> (SkillOutcome, Option<SkillMenu>) {
    match k.code {
        KeyCode::Esc | KeyCode::Enter => (SkillOutcome::Cancel, None),
        KeyCode::Up => {
            list.move_up();
            (SkillOutcome::Idle, Some(SkillMenu::List(list)))
        }
        KeyCode::Down => {
            list.move_down();
            (SkillOutcome::Idle, Some(SkillMenu::List(list)))
        }
        KeyCode::Left | KeyCode::Right if !list.entries.is_empty() => {
            let entry = &mut list.entries[list.selected];
            entry.enabled = !entry.enabled;
            let json = toggle_skill_json(&entry.name, entry.enabled);
            (SkillOutcome::Save(json), Some(SkillMenu::List(list)))
        }
        _ => (SkillOutcome::Idle, Some(SkillMenu::List(list))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::{Skill, config::SkillConfig};
    use std::path::PathBuf;

    fn key(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

    fn skill(name: &str, desc: &str) -> Skill {
        Skill { name: name.into(), description: desc.into(), body: String::new(), source: PathBuf::new() }
    }

    fn cfg_with(name: &str, enabled: bool) -> Config {
        let mut cfg = Config::default(); cfg.skills.insert(name.into(), SkillConfig { enabled }); cfg }

    #[test]
    fn from_discovered_merges_config_and_defaults_off() {
        let list = SkillList::from_discovered(&[skill("alpha", "a"), skill("beta", "b")], &cfg_with("beta", true));
        assert!(!list.entries[0].enabled, "alpha: missing from config -> OFF");
        assert!(list.entries[1].enabled, "beta: config ON honored");
        assert_eq!(list.entries[0].description, "a", "description carried over");
    }

    #[test]
    fn move_up_down_wrap() {
        let mut list = SkillList::from_discovered(&[skill("a", "1"), skill("b", "2"), skill("c", "3")], &Config::default());
        list.move_up();
        assert_eq!(list.selected, 2, "up from 0 wraps to last");
        list.move_down();
        assert_eq!(list.selected, 0, "down from last wraps to 0");
    }

    #[test]
    fn toggle_json_shape_on_and_off() {
        assert_eq!(toggle_skill_json("demo", true), json!({"skills": {"demo": {"enabled": true}}}));
        assert_eq!(toggle_skill_json("demo", false), json!({"skills": {"demo": {"enabled": false}}}));
    }

    #[test]
    fn left_arrow_toggles_selected_and_stays_open() {
        let list = SkillList::from_discovered(&[skill("srv", "d")], &cfg_with("srv", false));
        let (outcome, next) = handle_key(list, key(KeyCode::Left));
        match outcome {
            SkillOutcome::Save(json) => assert_eq!(json["skills"]["srv"]["enabled"], true),
            _ => panic!("expected Save"),
        }
        match next {
            Some(SkillMenu::List(l)) => assert!(l.entries[0].enabled && l.selected_entry().unwrap().enabled),
            _ => panic!("expected List to stay open"),
        }
    }

    #[test]
    fn enter_esc_close_and_empty_list_keys_are_noops() {
        for code in [KeyCode::Enter, KeyCode::Esc] {
            let (outcome, next) = handle_key(SkillList::from_discovered(&[skill("srv", "d")], &Config::default()), key(code));
            assert!(matches!(outcome, SkillOutcome::Cancel) && next.is_none());
        }
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right] {
            let (outcome, next) = handle_key(SkillList::from_discovered(&[], &Config::default()), key(code));
            assert!(matches!(outcome, SkillOutcome::Idle) && matches!(next, Some(SkillMenu::List(_))));
        }
    }
}
