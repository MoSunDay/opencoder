//! Topic execution state machine. Every step's position is derived from the
//! NFS layout (see `cursor`), so `run_topic` doubles as resume. Terminal
//! transitions (`finish`) write topic metadata + flip the store's
//! `team_topic_runs` rows in one logical step.

use std::sync::Arc;

use anyhow::{Context, Result};
use opencoder_core::message::now_ms;
use opencoder_store::Store;

use crate::config::{CancelToken, TeamRunConfig};
use crate::cursor::{self, Cursor, Stage};
use crate::decide::ask_json;
use crate::dispatcher::TeamDispatcher;
use crate::fs_store;
use crate::terminal;
use crate::types::*;

/// Per-invocation context threaded through the stage functions (plain data).
struct TopicCtx<'a> {
    store: &'a Arc<dyn Store>,
    dispatcher: &'a dyn TeamDispatcher,
    cfg: &'a TeamRunConfig,
    topic_id: &'a str,
}

/// Thin wrapper so stage functions read as single-line terminal calls.
async fn finish(
    ctx: &TopicCtx<'_>,
    meta: &mut TopicMeta,
    reason: &str,
    final_summary: Option<String>,
) -> Result<TopicMeta> {
    terminal::finish(ctx.store, &ctx.cfg.team_root, meta, reason, final_summary).await
}

/// Captain JSON decision bound to this topic's ledger scope.
async fn captain_ask<T, F>(
    ctx: &TopicCtx<'_>,
    node_id: &str,
    prompt: &str,
    validate: F,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
    F: Fn(&T) -> Result<()>,
{
    ask_json(
        ctx.dispatcher,
        Some(ctx.topic_id),
        node_id,
        prompt,
        validate,
    )
    .await
}

/// Open a new topic: snapshot the team's captain/membership, verify every
/// node is registered, write the initial `executing` topic metadata.
pub async fn start_topic(
    store: Arc<dyn Store>,
    cfg: &TeamRunConfig,
    team_name: &str,
    title: &str,
    requirement: &str,
) -> Result<TopicMeta> {
    let team = fs_store::load_team(&cfg.team_root, team_name)?;
    let members: Vec<MemberRef> = team
        .members
        .iter()
        .map(|m| MemberRef {
            node_id: m.node_id.clone(),
            name: m.name.clone(),
        })
        .collect();
    for member in std::iter::once(&team.captain).chain(members.iter()) {
        store
            .get_node(&member.node_id)
            .await
            .with_context(|| format!("node lookup for {}", member.node_id))?
            .with_context(|| format!("team member {} is not a registered node", member.node_id))?;
    }
    fs_store::init_topic(
        &cfg.team_root,
        team_name,
        title,
        requirement,
        team.captain.clone(),
        members,
        now_ms(),
    )
}

/// Execute (or resume) a topic to a terminal state. Returns the final
/// `TopicMeta` — including `finished(error)` outcomes, which stay resumable.
pub async fn run_topic(
    store: Arc<dyn Store>,
    dispatcher: Arc<dyn TeamDispatcher>,
    cfg: &TeamRunConfig,
    team_name: &str,
    topic_id: &str,
    cancel: CancelToken,
) -> Result<TopicMeta> {
    let mut meta = fs_store::load_topic(&cfg.team_root, team_name, topic_id)?;
    if meta.status == TOPIC_FINISHED && meta.finish_reason.as_deref() != Some(FINISH_ERROR) {
        return Ok(meta); // terminal and not resumable: idempotent no-op
    }
    if meta.status != TOPIC_EXECUTING {
        meta.status = TOPIC_EXECUTING.to_string();
        meta.finish_reason = None;
        meta.finished_at = None;
        fs_store::save_topic(&cfg.team_root, &meta)?;
    }
    let ctx = TopicCtx {
        store: &store,
        dispatcher: dispatcher.as_ref(),
        cfg,
        topic_id,
    };
    let member_ids: Vec<String> = meta.members.iter().map(|m| m.node_id.clone()).collect();
    let mut hint: Option<String> = None;
    // Set by a "continue" closing verdict: the next iteration MUST run the
    // plan stage for `turn + 1` even though the cursor (plan(T) exists, turn
    // T recorded) would say "closing". A crash in between simply re-asks the
    // closing decision — nothing was written, so it stays safe.
    let mut pending_plan: Option<usize> = None;
    loop {
        if cancel.is_cancelled() {
            return terminal::finish(
                ctx.store,
                &ctx.cfg.team_root,
                &mut meta,
                FINISH_CANCELLED,
                None,
            )
            .await;
        }
        let done: Vec<usize> = meta.turns.iter().map(|t| t.turn).collect();
        let cursor = match pending_plan.take() {
            Some(turn) => Cursor {
                turn,
                stage: Stage::Plan,
            },
            None => cursor::derive(
                &cfg.team_root,
                team_name,
                topic_id,
                &done,
                cfg.max_sub_turns,
            )?,
        };
        let terminal = match cursor.stage {
            Stage::Plan => stage_plan(&ctx, &mut meta, &member_ids, &mut hint, cursor.turn).await?,
            Stage::Sub { sub } => {
                stage_sub(&ctx, &mut meta, &member_ids, &cancel, sub, cursor.turn).await?
            }
            Stage::Closing => {
                stage_closing(&ctx, &mut meta, &mut hint, &mut pending_plan, cursor.turn).await?
            }
        };
        if let Some(meta_out) = terminal {
            return Ok(meta_out);
        }
    }
}

mod stages;

use stages::{stage_closing, stage_plan, stage_sub};
