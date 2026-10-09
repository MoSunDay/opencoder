//! A sandbox session owns a native session store in its existing writable
//! directory. The host only admits inputs and relays persisted output; the
//! container's native loop claims/records inputs with the usual transaction rules.
use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::fleet::RpcReply;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, Store};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

pub(crate) fn directory(worker: &Worker, id: &str, legacy: bool) -> Result<PathBuf> {
    Ok(super::sandbox_workflow_root(worker, legacy)?
        .join(id)
        .join("session"))
}

pub(crate) async fn open(
    worker: &Worker,
    record: &Record,
    legacy: bool,
) -> Result<Arc<LibsqlStore>> {
    let id = &record.assignment.index.id;
    let mut stores = worker.inner.sandbox_stores.lock().await;
    if let Some(store) = stores.get(id).and_then(std::sync::Weak::upgrade) {
        return Ok(store);
    }
    let dir = directory(worker, id, legacy)?;
    std::fs::create_dir_all(&dir)?;
    let store = Arc::new(LibsqlStore::open(dir.join("runtime.db")).await?);
    if store.get_session(id).await?.is_none() {
        super::super::agent::create_session(
            worker,
            id,
            record.assignment.request.target.as_deref().unwrap_or("act"),
            record.assignment.request.input["model"]
                .as_str()
                .map(str::to_owned),
            record.assignment.index.created_at,
            &worker.inner.state.workdir,
            super::super::agent::SessionLabels {
                title: record.assignment.request.input["title"]
                    .as_str()
                    .map(str::to_owned),
                kind: Some("agent".into()),
            },
        )
        .await?;
        let meta = worker
            .inner
            .state
            .store
            .get_session(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("sandbox session missing"))?;
        store.create_session(&meta).await?;
    }
    // Recover safely if initialization was interrupted after the session row.
    // Existing native messages and stable input receipts prevent reseeding a turn.
    if store.load_messages(id).await?.is_empty() {
        let prior = worker.inner.state.store.load_messages(id).await?;
        if !prior.is_empty() {
            store.append_messages(id, &prior).await?;
        } else {
            let input = &record.assignment.request.input;
            if input["prompt"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
            {
                let mut first = input.clone();
                if first["input_id"].is_null() {
                    first["input_id"] = json!("initial");
                }
                first["delivery"] = json!("steer");
                let receipt = admit(store.as_ref(), id, &first).await?;
                anyhow::ensure!(
                    receipt.status < 300,
                    "sandbox initial input: {}",
                    receipt.body
                );
            }
        }
    }
    stores.retain(|_, store| store.strong_count() > 0);
    stores.insert(id.clone(), Arc::downgrade(&store));
    Ok(store)
}

pub(crate) async fn admit(store: &dyn Store, id: &str, body: &Value) -> Result<RpcReply> {
    let Some(prompt) = body["prompt"].as_str().filter(|s| !s.trim().is_empty()) else {
        return Ok(RpcReply::error(400, "prompt is required"));
    };
    let Some(delivery) = Delivery::parse(body["delivery"].as_str().unwrap_or("steer")) else {
        return Ok(RpcReply::error(400, "delivery must be steer or queue"));
    };
    let input_id = body["input_id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| ulid::Ulid::new().to_string());
    if input_id.is_empty() || input_id.len() > 128 {
        return Ok(RpcReply::error(400, "invalid input_id"));
    }
    let images: Vec<String> = match body.get("images") {
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(images) => images,
            Err(_) => return Ok(RpcReply::error(400, "images must be an array of strings")),
        },
        None => vec![],
    };
    let input = SessionInput {
        seq: None,
        id: input_id,
        session_id: id.into(),
        delivery,
        prompt: prompt.into(),
        images,
        display_text: body["display"].as_str().map(str::to_owned),
        admitted_seq: 0,
        promoted_seq: None,
    };
    match store.admit_input_once(&input).await {
        Ok(receipt) => Ok(RpcReply::ok(
            json!({"id":id,"seq":receipt.seq,"input_id":input.id,"inserted":receipt.inserted,"status":"accepted"}),
        )),
        Err(error)
            if error
                .downcast_ref::<opencoder_store::InputConflict>()
                .is_some() =>
        {
            Ok(RpcReply::error(409, error.to_string()))
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn pending(store: &dyn Store, id: &str) -> Result<bool> {
    Ok(!store.pending_inputs(id, Delivery::Steer).await?.is_empty()
        || !store.pending_inputs(id, Delivery::Queue).await?.is_empty())
}

pub(crate) async fn sync_messages(worker: &Worker, store: &dyn Store, id: &str) -> Result<()> {
    let source = store.load_messages(id).await?;
    let saved = worker.inner.state.store.load_messages(id).await?;
    anyhow::ensure!(
        source.len() >= saved.len(),
        "sandbox transcript is behind the saved transcript"
    );
    if source.len() > saved.len() {
        worker
            .inner
            .state
            .store
            .append_messages(id, &source[saved.len()..])
            .await?;
    }
    Ok(())
}

pub(crate) async fn handle(
    store: &dyn Store,
    id: &str,
    method: &str,
    tail: &str,
    body: &Value,
) -> Result<RpcReply> {
    let path = tail.split('?').next().unwrap_or(tail);
    if path == "inputs" && method == "GET" {
        #[derive(serde::Deserialize)]
        struct Query {
            delivery: Option<String>,
        }
        let uri = format!("/{tail}").parse::<axum::http::Uri>()?;
        let Ok(query) = axum::extract::Query::<Query>::try_from_uri(&uri) else {
            return Ok(RpcReply::error(400, "invalid inputs query"));
        };
        let Some(delivery) = Delivery::parse(query.delivery.as_deref().unwrap_or("steer")) else {
            return Ok(RpcReply::error(400, "delivery must be steer or queue"));
        };
        let inputs: Vec<_> = store.pending_inputs(id, delivery).await?.iter().map(|input| {
            json!({"seq":input.seq,"delivery":input.delivery.as_str(),"prompt":input.prompt,
                "admitted_seq":input.admitted_seq,"promoted_seq":input.promoted_seq,"images":input.images.len()})
        }).collect();
        return Ok(RpcReply::ok(json!({"inputs":inputs})));
    }
    if path == "inputs/reorder" && method == "POST" {
        let (Some(a), Some(b)) = (body["a"].as_i64(), body["b"].as_i64()) else {
            return Ok(RpcReply::error(400, "a and b input sequences are required"));
        };
        store.swap_input_order(id, a, b).await?;
        return Ok(RpcReply::ok(json!({"ok":true})));
    }
    if let Some(seq) = path
        .strip_prefix("inputs/")
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|_| method == "DELETE")
    {
        let mut inputs = store.pending_inputs(id, Delivery::Steer).await?;
        inputs.extend(store.pending_inputs(id, Delivery::Queue).await?);
        if !inputs.iter().any(|input| input.seq == Some(seq)) {
            return Ok(RpcReply::error(404, "input is not pending"));
        }
        store.delete_input(seq).await?;
        return Ok(RpcReply::ok(json!({"ok":true})));
    }
    Ok(RpcReply::error(400, "unsupported sandbox input operation"))
}
