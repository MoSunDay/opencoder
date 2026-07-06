//! Background worker command processing — shared by the main worker and the
//! `/task`-spawned worker to avoid duplicate match arms.

use std::sync::Arc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::ChatClient;
use opencode_session::{run as run_session, SessionEvent, SessionState};
use tokio::sync::mpsc;

pub enum UiCmd {
    Prompt(String),
    SwitchAgent(String),
    /// Switch agent then immediately start a turn without recording a new user
    /// message. Used for the plan->act manual transition: the system prompt
    /// changes to act and the model reads the plan from conversation history.
    SwitchAndStart(String),
    /// Manually trigger conversation compaction.
    Compact,
    SetSkill(Option<String>),
    /// Hot-reload config at the next turn boundary. Sent by the `/model` menu.
    ReloadConfig(Config),
    Quit,
}

pub enum UiEvent {
    Session(SessionEvent),
    TurnDone,
}

/// Process one UI command against a session. Returns `true` when the worker
/// loop should break (Quit).
pub async fn process_cmd(
    cmd: UiCmd,
    sess: &mut SessionState,
    evt_tx: &mpsc::Sender<UiEvent>,
) -> bool {
    match cmd {
        UiCmd::Prompt(prompt) => {
            let tx = evt_tx.clone();
            let res = run_session(sess, prompt, move |sev| {
                let _ = tx.try_send(UiEvent::Session(sev));
            }).await;
            if let Err(e) = res {
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!("{e:#}"))));
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::SwitchAgent(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::AgentSwitch(name)));
            }
        }
        UiCmd::SwitchAndStart(name) => {
            if let Some(a) = resolve_agent(&name) {
                sess.agent = a;
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::AgentSwitch(name)));
            }
            let tx = evt_tx.clone();
            let res = run_session(sess, String::new(), move |sev| {
                let _ = tx.try_send(UiEvent::Session(sev));
            }).await;
            if let Err(e) = res {
                let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(format!("{e:#}"))));
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::Compact => {
            let registry = opencode_session::tools::registry();
            match opencode_session::compaction::compact(sess, &registry).await {
                Ok(summary) => {
                    let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Compaction(summary)));
                }
                Err(e) => {
                    let _ = evt_tx.try_send(UiEvent::Session(SessionEvent::Error(
                        format!("compaction failed: {e:#}"))));
                }
            }
            let _ = evt_tx.try_send(UiEvent::TurnDone);
        }
        UiCmd::SetSkill(body) => { sess.skill_prompt = body; }
        UiCmd::ReloadConfig(new_cfg) => {
            let api_key = new_cfg.api_key().unwrap_or_default();
            if let Ok(new_client) = ChatClient::new(&new_cfg.provider.base_url, &api_key) {
                sess.apply_config_reload(new_cfg, Arc::new(new_client));
            }
        }
        UiCmd::Quit => return true,
    }
    false
}
