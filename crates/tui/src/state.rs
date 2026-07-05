//! Multi-session state: each tab is an independent conversation with its own
//! ChatView, input, worker task, and event channel.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use opencode_core::{resolve_agent, Config};
use opencode_llm::ChatStream;
use opencode_session::{SessionEvent, SessionState};
use opencode_store::Store;

use crate::chat::ChatView;

/// Commands sent to a session's background worker.
pub(crate) enum UiCmd {
    Prompt(String),
    SwitchAgent(String),
    SetSkill(Option<String>),
    Quit,
}

/// Events received from session workers or the UI itself.
pub(crate) enum UiEvent {
    Session(SessionEvent),
    TurnDone,
}

/// One independent conversation tab.
pub(crate) struct SessionTab {
    pub id: String,
    pub title: String,
    pub chat: ChatView,
    pub cmd_tx: mpsc::Sender<UiCmd>,
    pub running: bool,
    pub scroll: u16,
    pub follow: bool,
    pub input: String,
    pub cursor_idx: usize,
    pub history: Vec<String>,
    pub hist_idx: Option<usize>,
    pub steer_count: u32,
    pub queue_count: u32,
    pub sys_tokens: u64,
    pub context_used: u64,
    pub active_skill: Option<String>,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl SessionTab {
    /// Create a new tab and spawn its background worker. The worker owns the
    /// `SessionState` and processes commands, forwarding session events through
    /// the returned receiver.
    pub(crate) fn spawn(
        id: String,
        session: SessionState,
        cancel: tokio_util::sync::CancellationToken,
    ) -> (Self, mpsc::Receiver<UiEvent>) {
        let session = session.with_cancel(cancel.clone());
        let agent_name = session.agent.name.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
        let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>(512);

        let chat = ChatView { agent: agent_name.clone(), ..Default::default() };
        let sys_tokens = crate::app::sys_tokens_for(&agent_name, &session.working_dir, None);

        tokio::spawn(async move {
            let mut sess = session;
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    UiCmd::Prompt(prompt) => {
                        let tx = evt_tx.clone();
                        let res = opencode_session::run(&mut sess, prompt, move |sev| {
                            let _ = tx.try_send(UiEvent::Session(sev));
                        }).await;
                        if let Err(e) = res {
                            let _ = evt_tx.try_send(UiEvent::Session(
                                SessionEvent::Error(format!("{e:#}")),
                            ));
                        }
                        let _ = evt_tx.try_send(UiEvent::TurnDone);
                    }
                    UiCmd::SwitchAgent(name) => {
                        if let Some(a) = resolve_agent(&name) {
                            sess.agent = a;
                            let _ = evt_tx.try_send(UiEvent::Session(
                                SessionEvent::AgentSwitch(name),
                            ));
                        }
                    }
                    UiCmd::SetSkill(body) => {
                        sess.skill_prompt = body;
                    }
                    UiCmd::Quit => break,
                }
            }
        });

        let tab = SessionTab {
            id,
            title: String::new(),
            chat,
            cmd_tx,
            running: false,
            scroll: 0,
            follow: true,
            input: String::new(),
            cursor_idx: 0,
            history: Vec::new(),
            hist_idx: None,
            steer_count: 0,
            queue_count: 0,
            sys_tokens,
            context_used: 0,
            active_skill: None,
            cancel,
        };

        (tab, evt_rx)
    }

    /// Push a styled marker line into the chat.
    pub(crate) fn push_marker(&mut self, line: ratatui::text::Line<'static>) {
        self.chat.push_marker(line);
    }
}

/// Format a short title from the first user message.
pub(crate) fn title_from_prompt(prompt: &str) -> String {
    let t = prompt.trim();
    if t.chars().count() <= 40 {
        t.to_string()
    } else {
        format!("{}...", t.chars().take(40).collect::<String>())
    }
}
