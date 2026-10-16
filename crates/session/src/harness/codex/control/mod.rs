//! Opt-in external input delivery, separate from the observed provider turn.
mod invoke;
mod journal;
use crate::{SessionEvent, SessionState};
use anyhow::{ensure, Result};
use journal::Request;
use opencoder_core::Message;
use opencoder_store::Delivery;
use std::time::Duration;

pub(crate) fn enabled(session: &SessionState) -> bool {
    session.harness.harness == opencoder_core::harness::Harness::Codex
        && session
            .harness
            .envs
            .get("OPENCODER_EXTERNAL_INPUT_PROTOCOL")
            .is_some_and(|s| s == "1")
        && session
            .harness
            .codex
            .as_ref()
            .is_some_and(|s| !s.startup_script.is_empty())
}

pub(crate) fn forwarded(id: &str) -> bool {
    id.starts_with("external-input-")
}

async fn deliver(
    session: &mut SessionState,
    request: &Request,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<bool> {
    let receipt = journal::root(session)?.join(format!("{}.receipt.json", request.seq));
    let reply = if receipt.exists() {
        serde_json::from_slice::<invoke::Reply>(&std::fs::read(&receipt)?)?
    } else if let Some(message) = &request.local_rejection {
        invoke::Reply {
            input_id: request.input_id.clone(),
            status: "rejected".into(),
            message: message.clone(),
            evidence: serde_json::Value::Null,
        }
    } else {
        match invoke::call(session, request).await {
            Ok(reply) => reply,
            Err(_) => {
                on_event(SessionEvent::Status(
                    "External input delivery is unconfirmed; retaining its original identity"
                        .into(),
                ));
                return Ok(false);
            }
        }
    };
    ensure!(
        reply.input_id == request.input_id,
        "external input receipt changed identity"
    );
    if reply.status == "pending" {
        return Ok(false);
    }
    ensure!(
        matches!(reply.status.as_str(), "accepted" | "rejected"),
        "invalid external input receipt"
    );
    journal::write(&receipt, &reply)?;
    if !session.messages.iter().any(|m| m.id == request.input_id) {
        session
            .record_checked(Message::user_with_display(
                request.input_id.clone(),
                request.prompt.clone(),
                Some(request.original.clone()),
                &request.images,
            ))
            .await?;
    }
    if reply.status == "rejected" {
        let text = format!(
            "Input was not delivered to the active turn: {}",
            reply.message
        );
        let id = format!("external-rejection-{}", request.seq);
        if !session.messages.iter().any(|m| m.id == id) {
            let mut message = Message::assistant(id);
            message
                .blocks
                .push(opencoder_core::ContentBlock::text(text.clone()));
            session.record_checked(message).await?;
        }
        on_event(SessionEvent::Status(text));
    }
    if let Some(store) = &session.store {
        store
            .mark_inputs_recorded(&session.id, &[request.seq])
            .await?;
    }
    on_event(SessionEvent::SteerConsumed {
        seq: request.seq,
        text: request.original.clone(),
    });
    Ok(true)
}

/// Entry recovery must reconcile a prior external send instead of making a followup.
pub(crate) async fn recover(
    session: &mut SessionState,
    seq: i64,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<bool> {
    if !enabled(session) || session.harness.resource_root.is_none() {
        return Ok(false);
    }
    let Some(request) = journal::load(session, seq)? else {
        return Ok(false);
    };
    loop {
        if deliver(session, &request, on_event).await? {
            return Ok(true);
        }
        ensure!(
            !session.cancel.as_ref().is_some_and(|c| c.is_cancelled()),
            "external input recovery interrupted; original identity preserved"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[derive(Default)]
pub(super) struct Pending {
    requests: Vec<Request>,
}

impl Pending {
    pub async fn poll(
        &mut self,
        session: &mut SessionState,
        on_event: &mut (dyn FnMut(SessionEvent) + Send),
    ) -> Result<()> {
        if self.requests.is_empty() {
            if let Some(store) = session.store.clone() {
                let rows = store.pending_inputs(&session.id, Delivery::Steer).await?;
                if !rows.is_empty() {
                    // Do not cancel this mutating transaction midway through promotion.
                    let limit = rows.iter().map(|r| r.admitted_seq).max().unwrap_or(0);
                    let claimed = store
                        .promote_inputs(&session.id, limit, Delivery::Steer)
                        .await?;
                    for row in rows
                        .iter()
                        .filter(|r| r.seq.is_some_and(|s| claimed.contains(&s)))
                    {
                        self.requests.push(
                            journal::prepare(session, row.seq.unwrap(), &row.prompt, &row.images)
                                .await?,
                        );
                    }
                }
            }
        }
        if let Some(request) = self.requests.first().cloned() {
            if deliver(session, &request, on_event).await? {
                self.requests.remove(0);
            }
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

pub(super) async fn cancel(
    session: &SessionState,
    on_event: &mut (dyn FnMut(SessionEvent) + Send),
) -> Result<()> {
    let Some(parent) = &session.harness.last_input_id else {
        return Ok(());
    };
    let request = Request {
        schema_version: 1,
        action: "cancel".into(),
        input_id: format!("cancel-{parent}"),
        parent_input_id: parent.clone(),
        seq: 0,
        original: String::new(),
        prompt: String::new(),
        images: vec![],
        local_rejection: None,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match invoke::call(session, &request).await {
            Ok(reply) if reply.status == "accepted" => return Ok(()),
            _ if tokio::time::Instant::now() >= deadline => {
                on_event(SessionEvent::Status(
                    "Remote cancellation remains unconfirmed; inspect the original control receipt"
                        .into(),
                ));
                return Ok(());
            }
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}
