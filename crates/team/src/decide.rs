//! Captain-decision plumbing shared by the runtime: dispatch-with-retry,
//! parse+validate-with-correction (todos/parent.rs style), validators and
//! the small pure derivations (member prompts, digests, result records).

use std::path::Path;

use anyhow::{bail, Result};
use opencoder_core::message::now_ms;
use serde::de::DeserializeOwned;

use crate::dispatcher::TeamDispatcher;
use crate::fs_store;
use crate::prompts;
use crate::types::*;

/// Correction re-asks after an unparsable/invalid captain reply (3 asks max).
pub const PARSE_RETRIES: usize = 2;
/// Transport-level retries for a captain dispatch (2 attempts max).
pub const ASK_RETRIES: usize = 1;

async fn ask_with_retry(
    dispatcher: &dyn TeamDispatcher,
    topic: Option<&str>,
    node_id: &str,
    prompt: &str,
) -> Result<String> {
    let mut attempts = ASK_RETRIES + 1;
    loop {
        match dispatcher.ask(topic, node_id, prompt).await {
            Ok(text) => return Ok(text),
            Err(error) if attempts > 1 => {
                attempts -= 1;
                tracing::warn!(node = node_id, error = %format!("{error:#}"), attempts, "team dispatch failed; retrying");
            }
            Err(error) => return Err(error),
        }
    }
}

/// Ask a node for a JSON decision: parse + validate, re-ask with a
/// correction prompt up to `PARSE_RETRIES` times.
pub async fn ask_json<T, F>(
    dispatcher: &dyn TeamDispatcher,
    topic: Option<&str>,
    node_id: &str,
    prompt: &str,
    validate: F,
) -> Result<T>
where
    T: DeserializeOwned,
    F: Fn(&T) -> Result<()>,
{
    let mut retries = PARSE_RETRIES;
    let mut current = prompt.to_string();
    loop {
        let raw = ask_with_retry(dispatcher, topic, node_id, &current).await?;
        match parse_decision(&raw).and_then(|value| validate(&value).map(|()| value)) {
            Ok(value) => return Ok(value),
            Err(error) if retries > 0 => {
                retries -= 1;
                tracing::warn!(node = node_id, error = %format!("{error:#}"), retries, "decision unparsable; re-asking");
                current = prompts::correction_prompt(&format!("{error:#}"), &raw);
            }
            Err(error) => {
                return Err(error.context("team decision unusable after correction retries"))
            }
        }
    }
}

/// The prompt + result kind for one member dispatch in sub-turn `sub`.
pub fn member_prompt(
    meta: &TopicMeta,
    plan: &PlanRecord,
    sub: usize,
    prev_summary: Option<&SummaryRecord>,
    node_id: &str,
) -> (String, &'static str) {
    if sub == 0 {
        (
            prompts::answer_prompt(&meta.requirement, &plan.question, node_id),
            RESULT_ANSWER,
        )
    } else {
        let prev = prev_summary.expect("alignment sub-turns follow a summary");
        let question = prev
            .ambiguities
            .iter()
            .find(|a| a.node_id == node_id)
            .map(|a| a.question.clone())
            .unwrap_or_else(|| plan.question.clone());
        (
            prompts::alignment_prompt(&meta.requirement, &plan.question, &prev.summary, &question),
            RESULT_ALIGNMENT,
        )
    }
}

pub fn result_record(
    node_id: &str,
    turn: usize,
    sub_turn: usize,
    kind: &str,
    answer: String,
    error: Option<String>,
) -> ResultRecord {
    ResultRecord {
        node_id: node_id.to_string(),
        turn,
        sub_turn,
        kind: kind.to_string(),
        ok: error.is_none(),
        answer,
        error,
        created_at: now_ms(),
    }
}

pub fn validate_plan(decision: &PlanDecision, member_ids: &[String]) -> Result<()> {
    if decision.question.trim().is_empty() {
        bail!("question is empty");
    }
    if decision.participants.is_empty() {
        bail!("participants is empty");
    }
    if let Some(id) = decision
        .participants
        .iter()
        .find(|id| !member_ids.contains(id))
    {
        bail!("participant {id} is not a team member");
    }
    Ok(())
}

pub fn validate_summary(decision: &SummaryDecision) -> Result<()> {
    if decision.summary.trim().is_empty() {
        bail!("summary is empty");
    }
    Ok(())
}

pub fn validate_closing(decision: &ClosingDecision) -> Result<()> {
    if decision.complete
        && decision
            .final_summary
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
    {
        bail!("complete=true requires a non-empty final_summary");
    }
    Ok(())
}

pub fn validate_profile(_decision: &ProfileDecision) -> Result<()> {
    Ok(())
}

/// Order-preserving dedup (participants come from model output).
pub fn dedup(ids: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Keep only ambiguities that name a real member, dedup by node, drop empty
/// questions.
pub fn sanitize_ambiguities(ambiguities: Vec<Ambiguity>, member_ids: &[String]) -> Vec<Ambiguity> {
    ambiguities
        .into_iter()
        .filter(|a| member_ids.contains(&a.node_id) && !a.question.trim().is_empty())
        .fold(Vec::new(), |mut acc, a| {
            if !acc.iter().any(|x: &Ambiguity| x.node_id == a.node_id) {
                acc.push(a);
            }
            acc
        })
}

/// Members (with capabilities) for the plan prompt, restricted to the topic's
/// membership snapshot.
pub fn capability_table(
    team_root: &Path,
    team_name: &str,
    member_ids: &[String],
) -> Result<Vec<TeamMember>> {
    let team = fs_store::load_team(team_root, team_name)?;
    Ok(team
        .members
        .into_iter()
        .filter(|m| member_ids.contains(&m.node_id))
        .collect())
}

/// Completed turns as the captain sees them in later prompts.
pub fn turn_digests(team_root: &Path, meta: &TopicMeta) -> Result<Vec<prompts::TurnDigest>> {
    meta.turns
        .iter()
        .map(|t| {
            let summary = fs_store::read_summary(
                team_root,
                &meta.team_name,
                &meta.topic_id,
                t.turn,
                t.sub_turns.saturating_sub(1),
            )?
            .map(|s| s.summary)
            .unwrap_or_default();
            Ok(prompts::TurnDigest {
                turn: t.turn,
                question: t.question.clone(),
                aligned: t.aligned,
                summary,
            })
        })
        .collect()
}

pub fn collect_results(
    team_root: &Path,
    team_name: &str,
    topic_id: &str,
    turn: usize,
    sub_turn: usize,
    participants: &[String],
) -> Result<Vec<ResultRecord>> {
    participants
        .iter()
        .filter_map(|node_id| {
            fs_store::read_result(team_root, team_name, topic_id, turn, sub_turn, node_id)
                .transpose()
        })
        .collect()
}

pub fn last_summary_text(team_root: &Path, meta: &TopicMeta) -> Result<String> {
    let Some(last) = meta.turns.last() else {
        return Ok(String::new());
    };
    Ok(fs_store::read_summary(
        team_root,
        &meta.team_name,
        &meta.topic_id,
        last.turn,
        last.sub_turns.saturating_sub(1),
    )?
    .map(|s| s.summary)
    .unwrap_or_default())
}
