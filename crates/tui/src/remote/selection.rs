use super::client::{normalize_url, ServerClient};
use crate::{agent_menu::AgentCard, task::TaskPick};
use anyhow::{Context, Result};
use opencoder_core::{harness::RemoteSession, Config};

pub async fn catalog(config: &Config) -> Result<Vec<AgentCard>> {
    let mut cards = vec![AgentCard {
        name: "self".into(),
        description: "本地执行 · 新建任务".into(),
    }];
    if config.opencoder_server.enabled {
        cards.extend(
            ServerClient::configured(config)?
                .capabilities()
                .await?
                .into_iter()
                .map(|card| AgentCard {
                    name: card.id,
                    description: format!(
                        "{} · {} · {}",
                        card.kind.prefix(),
                        card.target,
                        card.summary
                    ),
                }),
        );
    }
    Ok(cards)
}

pub async fn select(config: &Config, name: &str) -> Result<TaskPick> {
    if name == "self" {
        return Ok(TaskPick::New);
    }
    let capability = ServerClient::configured(config)?
        .capabilities()
        .await?
        .into_iter()
        .find(|card| card.id == name)
        .context("Unknown Server capability; use /agent to select an Agent or Operator")?;
    Ok(TaskPick::Remote(RemoteSession {
        server_url: normalize_url(&config.opencoder_server.url)?,
        capability,
        created: false,
        initial_input: None,
    }))
}

pub(crate) type Selection = Option<tokio::sync::oneshot::Receiver<Result<TaskPick>>>;
pub(crate) fn request(
    config: Config,
    name: String,
) -> tokio::sync::oneshot::Receiver<Result<TaskPick>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(select(&config, &name).await);
    });
    rx
}
pub(crate) fn poll(request: &mut Selection) -> Option<Result<TaskPick>> {
    let receiver = request.as_mut()?;
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return None,
        Err(error) => Err(error.into()),
    };
    *request = None;
    Some(result)
}

pub(crate) fn poll_ui(
    menu: &mut Option<crate::agent_menu::AgentMenu>,
    request: &mut Selection,
    flash: &mut Option<(String, u32)>,
    tick: u32,
) -> (bool, Option<TaskPick>) {
    let mut changed = false;
    if let Some(result) = menu.as_mut().and_then(|menu| menu.poll_catalog()) {
        changed = true;
        if let Err(error) = result {
            *flash = Some((format!("{error:#}"), tick));
        }
    }
    let mut pick = None;
    if let Some(result) = poll(request) {
        changed = true;
        match result {
            Ok(selected) => pick = Some(selected),
            Err(error) => *flash = Some((format!("{error:#}"), tick)),
        }
    }
    (changed, pick)
}
