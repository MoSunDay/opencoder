use super::{client::ServerClient, stream, transcript};
use crate::worker::{UiCmd, UiEvent};
use anyhow::{Context, Result};
use opencoder_core::{
    fleet::{CreateExecution, ExecutionIndex, ExecutionStatus},
    harness::RemoteSession,
};
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{SessionPatch, Store};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) fn spawn(
    session: SessionState,
    mut commands: mpsc::Receiver<UiCmd>,
    ui: mpsc::Sender<UiEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let id = session.id.clone();
        let Some(store) = session.store.clone() else {
            return;
        };
        let Some(mut remote) = session.harness.remote.clone() else {
            return;
        };
        let client =
            match ServerClient::new(&remote.server_url, session.config.network.proxy.as_deref()) {
                Ok(client) => client,
                Err(error) => {
                    report(&ui, &remote, error).await;
                    return;
                }
            };
        let detach = CancellationToken::new();
        let _detach_on_drop = detach.clone().drop_guard();
        let questions = super::questions::QuestionBridge {
            client: client.clone(),
            execution: id.clone(),
            hub: session.question_hub.clone(),
            ui: ui.clone(),
            detach: detach.clone(),
        };
        let (frames_tx, mut frames_rx) = mpsc::channel(256);
        let mut subscription = None;
        let mut cancel = session.cancel.clone().unwrap_or_default();
        let mut cancel_sent = false;
        let mut recover = remote.created || remote.initial_input.is_some();
        let mut retry = tokio::time::interval(std::time::Duration::from_secs(2));
        if remote.created || remote.initial_input.is_some() {
            if let Err(error) = ensure_created(&client, &store, &id, &mut remote).await {
                report(&ui, &remote, error).await;
            } else {
                match hydrate(&client, &id, &remote, &ui).await {
                    Ok(cursor) => {
                        recover = false;
                        subscription = Some(tokio::spawn(stream::subscribe(
                            client.clone(),
                            id.clone(),
                            cursor,
                            frames_tx.clone(),
                            detach.clone(),
                        )))
                    }
                    Err(error) => report(&ui, &remote, error).await,
                }
            }
        }
        if remote.created {
            questions.restore().await;
        }
        loop {
            tokio::select! {
                _ = retry.tick(), if recover => {
                    if ensure_created(&client,&store,&id,&mut remote).await.is_ok() {
                        if let Ok(cursor) = hydrate(&client,&id,&remote,&ui).await {
                            subscription = Some(tokio::spawn(stream::subscribe(client.clone(),id.clone(),cursor,frames_tx.clone(),detach.clone())));
                            questions.restore().await;
                            recover = false;
                        }
                    }
                }
                command = commands.recv() => match command {
                    None | Some(UiCmd::Quit) => break,
                    Some(UiCmd::ResetCancel(token)) => { cancel = token; cancel_sent = false; }
                    Some(UiCmd::Prompt(prompt, images)) => {
                        let result = async {
                            if recover {
                                ensure_created(&client,&store,&id,&mut remote).await?;
                                let cursor = hydrate(&client,&id,&remote,&ui).await?;
                                subscription = Some(tokio::spawn(stream::subscribe(client.clone(),id.clone(),cursor,frames_tx.clone(),detach.clone())));
                                recover = false;
                            }
                            if !remote.created {
                                let same_input = remote.initial_input.as_ref().is_none_or(|input| input["prompt"] == prompt && input["images"] == json!(images));
                                if remote.initial_input.is_none() {
                                    remote.initial_input = Some(json!({"prompt":prompt,"images":images,"literal_mentions":true}));
                                    save(&store, &id, &remote).await?;
                                }
                                ensure_created(&client, &store, &id, &mut remote).await?;
                                if !same_input {
                                    client.command(&id, "prompt", json!({"prompt":prompt,"images":images,"input_id":opencoder_session::runner::new_id()})).await?;
                                }
                            } else {
                                client.command(&id, "prompt", json!({"prompt":prompt,"images":images,"input_id":opencoder_session::runner::new_id()})).await?;
                            }
                            store.update_session(&id, &SessionPatch {updated_at:Some(opencoder_core::message::now_ms()), ..Default::default()}).await?;
                            Ok::<(), anyhow::Error>(())
                        }.await;
                        if let Err(error) = result { report(&ui, &remote, error).await; }
                        else if subscription.is_none() {
                            // Start from zero after first admission: no event may be
                            // skipped merely because Server ran before POST returned.
                            subscription = Some(tokio::spawn(stream::subscribe(client.clone(), id.clone(), 0, frames_tx.clone(), detach.clone())));
                        }
                    }
                    Some(_) => {
                        let _ = ui.send(UiEvent::Session(SessionEvent::Status("This control is available for /agent self tasks".into()))).await;
                    }
                },
                _ = cancel.cancelled(), if !cancel_sent => {
                    cancel_sent = true;
                    if remote.created {
                        if let Err(error) = client.command(&id,"interrupt",json!({})).await { report(&ui,&remote,error).await; }
                    }
                }
                frame = frames_rx.recv(), if subscription.is_some() => {
                    match frame {
                        Some(Ok(frame)) => {
                            if let Some(event) = SessionEvent::from_sse(&frame.kind, frame.data) {
                                if matches!(event, SessionEvent::AgentSwitch(_)) { continue; }
                                match &event {
                                    SessionEvent::ToolStart {id:call,name,input} if name == "question" => questions.start(call.clone(),input),
                                    SessionEvent::ToolEnd {id:call,..} => session.question_hub.abandon(call),
                                    _ => {}
                                }
                                if matches!(event, SessionEvent::TranscriptReset(_)) {
                                    if let Some(old)=subscription.take() {old.abort();}
                                    while frames_rx.try_recv().is_ok() {}
                                    match hydrate(&client,&id,&remote,&ui).await {
                                        Ok(cursor) => subscription=Some(tokio::spawn(stream::subscribe(client.clone(),id.clone(),cursor,frames_tx.clone(),detach.clone()))),
                                        Err(error) => {recover=true;report(&ui,&remote,error).await;}
                                    }
                                    continue;
                                }
                                if ui.send(UiEvent::Session(event)).await.is_err() { break; }
                            }
                        }
                        Some(Err(error)) => {
                            let _ = ui.send(UiEvent::Session(SessionEvent::Status(format!("Server disconnected; reconnecting: {error}")))).await;
                        }
                        None => break,
                    }
                }
            }
        }
        detach.cancel();
        if let Some(subscription) = subscription {
            subscription.abort();
        }
    })
}

async fn save(store: &Arc<dyn Store>, id: &str, remote: &RemoteSession) -> Result<()> {
    let mut runtime = store
        .harness_runtime(id)
        .await?
        .context("remote task bookmark missing")?;
    runtime.remote = Some(remote.clone());
    store.set_harness_runtime(id, &runtime).await
}

async fn ensure_created(
    client: &ServerClient,
    store: &Arc<dyn Store>,
    id: &str,
    remote: &mut RemoteSession,
) -> Result<()> {
    if remote.created {
        return Ok(());
    }
    let request = CreateExecution {
        id: id.into(),
        kind: remote.capability.kind,
        target: Some(remote.capability.target.clone()),
        input: remote
            .initial_input
            .clone()
            .context("remote first prompt missing")?,
        node_id: None,
    };
    client
        .post("/api/executions", &serde_json::to_value(request)?)
        .await?;
    remote.created = true;
    remote.initial_input = None;
    save(store, id, remote).await
}

async fn hydrate(
    client: &ServerClient,
    id: &str,
    remote: &RemoteSession,
    ui: &mpsc::Sender<UiEvent>,
) -> Result<i64> {
    let (events, cursor) = transcript::events(client, id).await?;
    let messages = transcript::messages(client, id).await?;
    let index: ExecutionIndex = client.get(&format!("/api/executions/{id}/index")).await?;
    let running = matches!(
        index.status,
        ExecutionStatus::Pending | ExecutionStatus::Running | ExecutionStatus::Cancelling
    );
    let chat = transcript::replay(&remote.label(), &messages, &events);
    ui.send(UiEvent::RemoteSnapshot {
        chat: Box::new(chat),
        running,
    })
    .await?;
    Ok(cursor)
}

async fn report(ui: &mpsc::Sender<UiEvent>, remote: &RemoteSession, error: anyhow::Error) {
    let _ = ui
        .send(UiEvent::Session(SessionEvent::Error(format!(
            "{}: {error:#}",
            remote.label()
        ))))
        .await;
}
