//! The three per-step stages of the topic loop. Each returns `None` to keep
//! looping (cursor advances from disk) or the terminal `TopicMeta` to stop.

use anyhow::{Context, Result};
use opencoder_core::message::now_ms;

use super::{captain_ask, finish, TopicCtx};
use crate::config::CancelToken;
use crate::decide::{
    capability_table, collect_results, dedup, last_summary_text, member_prompt, result_record,
    sanitize_ambiguities, turn_digests, validate_closing, validate_plan, validate_summary,
};
use crate::fs_store;
use crate::prompts;
use crate::types::*;

/// Decision ①. A failure finishes the topic with `error` (resumable).
pub(super) async fn stage_plan(
    ctx: &TopicCtx<'_>,
    meta: &mut TopicMeta,
    member_ids: &[String],
    hint: &mut Option<String>,
    turn: usize,
) -> Result<Option<TopicMeta>> {
    let history = turn_digests(&ctx.cfg.team_root, meta)?;
    let members = capability_table(&ctx.cfg.team_root, &meta.team_name, member_ids)?;
    let prompt = prompts::plan_prompt(&meta.requirement, &members, &history, hint.as_deref());
    let ids = member_ids.to_vec();
    let captain = meta.captain.node_id.clone();
    let decision = match captain_ask(ctx, &captain, &prompt, |d: &PlanDecision| {
        validate_plan(d, &ids)
    })
    .await
    {
        Ok(decision) => decision,
        Err(error) => {
            tracing::warn!(topic = ctx.topic_id, error = %format!("{error:#}"), "plan decision failed");
            return Ok(Some(finish(ctx, meta, FINISH_ERROR, None).await?));
        }
    };
    fs_store::write_plan(
        &ctx.cfg.team_root,
        &meta.team_name,
        ctx.topic_id,
        &PlanRecord {
            turn,
            question: decision.question,
            participants: dedup(decision.participants),
            rationale: decision.rationale,
        },
    )?;
    *hint = None;
    Ok(None)
}

/// Sub-turn execution: fill missing member results, decision ②, then either
/// record the turn, advance one sub-turn, or finish with `max_sub_turns`.
pub(super) async fn stage_sub(
    ctx: &TopicCtx<'_>,
    meta: &mut TopicMeta,
    member_ids: &[String],
    cancel: &CancelToken,
    sub: usize,
    turn: usize,
) -> Result<Option<TopicMeta>> {
    let plan = fs_store::read_turn_plan(&ctx.cfg.team_root, &meta.team_name, ctx.topic_id, turn)?
        .context("turn plan missing")?;
    let prev_summary = if sub == 0 {
        None
    } else {
        fs_store::read_summary(
            &ctx.cfg.team_root,
            &meta.team_name,
            ctx.topic_id,
            turn,
            sub - 1,
        )?
    };
    let participants = if sub == 0 {
        plan.participants.clone()
    } else {
        prev_summary
            .as_ref()
            .map(|s| s.ambiguities.iter().map(|a| a.node_id.clone()).collect())
            .context("previous sub-turn summary missing")?
    };
    let participants = dedup(participants);
    for node_id in &participants {
        let existing = fs_store::read_result(
            &ctx.cfg.team_root,
            &meta.team_name,
            ctx.topic_id,
            turn,
            sub,
            node_id,
        )?;
        if existing.is_some() {
            continue; // resume: this member already answered
        }
        if cancel.is_cancelled() {
            return Ok(Some(finish(ctx, meta, FINISH_CANCELLED, None).await?));
        }
        let (prompt, kind) = member_prompt(meta, &plan, sub, prev_summary.as_ref(), node_id);
        let rec = match ctx
            .dispatcher
            .ask(Some(ctx.topic_id), node_id, &prompt)
            .await
        {
            Ok(answer) => result_record(node_id, turn, sub, kind, answer, None),
            // Member failures are tolerated: the summary decision sees the
            // failure and the topic keeps moving.
            Err(error) => {
            tracing::warn!(node = node_id, error = %format!("{error:#}"), "member dispatch failed");
                result_record(
                    node_id,
                    turn,
                    sub,
                    kind,
                    String::new(),
                    Some(format!("{error:#}")),
                )
            }
        };
        fs_store::write_result(&ctx.cfg.team_root, &meta.team_name, ctx.topic_id, &rec)?;
    }
    let summary =
        match fs_store::read_summary(&ctx.cfg.team_root, &meta.team_name, ctx.topic_id, turn, sub)?
        {
            Some(existing) => existing, // crash between summary write and turn record
            None => {
                match summarize(ctx, meta, member_ids, &plan, &participants, turn, sub).await? {
                    SummaryOutcome::Record(record) => record,
                    SummaryOutcome::Terminated(meta_out) => return Ok(Some(*meta_out)),
                }
            }
        };
    let turn_meta = TopicTurnMeta {
        turn,
        question: plan.question.clone(),
        participants: plan.participants.clone(),
        aligned: summary.aligned,
        sub_turns: sub + 1,
    };
    if summary.aligned {
        meta.turns.push(turn_meta);
        fs_store::save_topic(&ctx.cfg.team_root, meta)?; // next loop: closing
    } else if sub < ctx.cfg.max_sub_turns {
        return Ok(None); // next alignment sub-turn
    } else {
        meta.turns.push(turn_meta);
        fs_store::save_topic(&ctx.cfg.team_root, meta)?;
        let final_text = summary.summary;
        return Ok(Some(
            finish(ctx, meta, FINISH_MAX_SUB_TURNS, Some(final_text)).await?,
        ));
    }
    Ok(None)
}

/// Outcome of a summary step: the record, or the terminal meta when the
/// captain decision failed and the topic was finished with `error`.
enum SummaryOutcome {
    Record(SummaryRecord),
    Terminated(Box<TopicMeta>),
}

/// Decision ② (summary): ask the captain over the on-disk results.
async fn summarize(
    ctx: &TopicCtx<'_>,
    meta: &mut TopicMeta,
    member_ids: &[String],
    plan: &PlanRecord,
    participants: &[String],
    turn: usize,
    sub: usize,
) -> Result<SummaryOutcome> {
    let results = collect_results(
        &ctx.cfg.team_root,
        &meta.team_name,
        ctx.topic_id,
        turn,
        sub,
        participants,
    )?;
    let prompt = prompts::summary_prompt(&meta.requirement, &plan.question, &results);
    let captain = meta.captain.node_id.clone();
    let decision = match captain_ask(ctx, &captain, &prompt, validate_summary).await {
        Ok(decision) => decision,
        Err(error) => {
            tracing::warn!(topic = ctx.topic_id, error = %format!("{error:#}"), "summary decision failed");
            let meta_out = finish(ctx, meta, FINISH_ERROR, None).await?;
            return Ok(SummaryOutcome::Terminated(Box::new(meta_out)));
        }
    };
    let record = SummaryRecord {
        summary: decision.summary,
        aligned: decision.aligned,
        ambiguities: sanitize_ambiguities(decision.ambiguities, member_ids),
        created_at: now_ms(),
    };
    fs_store::write_summary(
        &ctx.cfg.team_root,
        &meta.team_name,
        ctx.topic_id,
        turn,
        sub,
        &record,
    )?;
    Ok(SummaryOutcome::Record(record))
}

/// Decision ③: complete the topic (final summary) or open the next round.
/// `pending_plan` carries the "continue" verdict across the loop boundary.
pub(super) async fn stage_closing(
    ctx: &TopicCtx<'_>,
    meta: &mut TopicMeta,
    hint: &mut Option<String>,
    pending_plan: &mut Option<usize>,
    turn: usize,
) -> Result<Option<TopicMeta>> {
    let history = turn_digests(&ctx.cfg.team_root, meta)?;
    let prompt = prompts::closing_prompt(&meta.requirement, &history);
    let captain = meta.captain.node_id.clone();
    let decision = match captain_ask(ctx, &captain, &prompt, validate_closing).await {
        Ok(decision) => decision,
        Err(error) => {
            tracing::warn!(topic = ctx.topic_id, error = %format!("{error:#}"), "closing decision failed");
            return Ok(Some(finish(ctx, meta, FINISH_ERROR, None).await?));
        }
    };
    if decision.complete {
        let final_text = decision
            .final_summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| last_summary_text(&ctx.cfg.team_root, meta).unwrap_or_default());
        return Ok(Some(
            finish(ctx, meta, FINISH_COMPLETE, Some(final_text)).await?,
        ));
    }
    *hint = decision.next_question.filter(|q| !q.trim().is_empty());
    if turn >= ctx.cfg.max_turns {
        let final_text = last_summary_text(&ctx.cfg.team_root, meta)?;
        return Ok(Some(
            finish(ctx, meta, FINISH_MAX_TURNS, Some(final_text)).await?,
        ));
    }
    *pending_plan = Some(turn + 1); // force the plan stage even after a crash
    Ok(None)
}
