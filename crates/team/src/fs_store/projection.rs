use std::{fs::File, io::BufReader, path::Path};

use anyhow::{bail, Context, Result};
use serde::de::IgnoredAny;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{json_field::top_level_text, read_json, MAX_FILE_BYTES};
use crate::{
    layout,
    types::{MemberRef, PlanRecord, TopicMeta},
};

const INLINE_BYTES: u64 = 64 * 1024;

pub struct TopicTurnPage {
    pub turns: Vec<Value>,
    pub next_turn: Option<u64>,
}

#[derive(Deserialize)]
struct TopicSkeleton {
    topic_id: String,
    team_name: String,
    title: String,
    #[serde(rename = "requirement")]
    _requirement: IgnoredAny,
    status: String,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    finished_at: Option<i64>,
    captain: MemberRef,
    #[serde(default)]
    members: Vec<MemberRef>,
    #[serde(default)]
    turns: Vec<TurnSkeleton>,
    #[serde(default)]
    #[serde(rename = "final_summary")]
    _final_summary: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct TurnSkeleton {
    turn: usize,
    #[serde(rename = "question")]
    _question: IgnoredAny,
    #[serde(default)]
    participants: Vec<String>,
    #[serde(default)]
    aligned: bool,
    #[serde(default)]
    sub_turns: usize,
}

pub fn topic_summary(root: &Path, name: &str, id: &str) -> Result<Value> {
    let path = layout::topic_file(root, name, id)?;
    let bytes = checked_len(&path)?;
    if bytes <= INLINE_BYTES {
        let topic: TopicMeta = read_json(&path)?.context("topic not found")?;
        return Ok(small_summary(topic));
    }
    let topic = read_skeleton(&path)?;
    let mut turns = topic
        .turns
        .iter()
        .take(51)
        .map(|turn| turn_value(root, name, id, turn, bytes))
        .collect::<Result<Vec<_>>>()?;
    let more = turns.len() > 50;
    turns.truncate(50);
    let next_turn = more.then(|| turns.last().and_then(turn_number)).flatten();
    Ok(json!({
        "topic_id": topic.topic_id,
        "team_name": topic.team_name,
        "title": topic.title,
        "requirement": top_level_text(&path, "requirement", INLINE_BYTES)?,
        "status": topic.status,
        "finish_reason": topic.finish_reason,
        "created_at": topic.created_at,
        "finished_at": topic.finished_at,
        "captain": topic.captain,
        "members": topic.members,
        "turns": turns,
        "final_summary": top_level_text(&path, "final_summary", INLINE_BYTES)?,
        "turns_page": {"next_turn": next_turn, "more": more},
    }))
}

pub fn topic_turns_page(
    root: &Path,
    name: &str,
    id: &str,
    after: usize,
    limit: usize,
) -> Result<TopicTurnPage> {
    let path = layout::topic_file(root, name, id)?;
    let bytes = checked_len(&path)?;
    let source = if bytes <= INLINE_BYTES {
        read_json::<TopicMeta>(&path)?
            .context("topic not found")?
            .turns
            .into_iter()
            .map(|turn| json!(turn))
            .collect::<Vec<_>>()
    } else {
        read_skeleton(&path)?
            .turns
            .iter()
            .map(|turn| turn_value(root, name, id, turn, bytes))
            .collect::<Result<Vec<_>>>()?
    };
    let mut turns = source
        .into_iter()
        .filter(|turn| turn_number(turn).is_some_and(|number| number > after as u64))
        .take(limit + 1)
        .map(page_turn)
        .collect::<Vec<_>>();
    let more = turns.len() > limit;
    turns.truncate(limit);
    Ok(TopicTurnPage {
        next_turn: more.then(|| turns.last().and_then(turn_number)).flatten(),
        turns,
    })
}

fn checked_len(path: &Path) -> Result<u64> {
    let bytes = std::fs::metadata(path)
        .with_context(|| format!("read {} metadata", path.display()))?
        .len();
    if bytes > MAX_FILE_BYTES as u64 {
        bail!("{} is {bytes} bytes (max {MAX_FILE_BYTES})", path.display());
    }
    Ok(bytes)
}

fn read_skeleton(path: &Path) -> Result<TopicSkeleton> {
    let reader = BufReader::with_capacity(INLINE_BYTES as usize, File::open(path)?);
    serde_json::from_reader(reader).with_context(|| format!("malformed JSON in {}", path.display()))
}

fn small_summary(mut topic: TopicMeta) -> Value {
    let more = topic.turns.len() > 50;
    topic.turns.truncate(50);
    let next_turn = more
        .then(|| topic.turns.last().map(|turn| turn.turn as u64))
        .flatten();
    let mut value = serde_json::to_value(topic).expect("TopicMeta serialization cannot fail");
    value["turns_page"] = json!({"next_turn": next_turn, "more": more});
    value
}

fn turn_value(
    root: &Path,
    name: &str,
    id: &str,
    turn: &TurnSkeleton,
    topic_bytes: u64,
) -> Result<Value> {
    let path = layout::plan_file(root, name, id, turn.turn)?;
    let plan = match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= INLINE_BYTES => read_json::<PlanRecord>(&path)?,
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(plan) = plan {
        Ok(json!({
            "turn": turn.turn,
            "question": plan.question,
            "participants": turn.participants,
            "aligned": turn.aligned,
            "sub_turns": turn.sub_turns,
        }))
    } else {
        Ok(json!({
            "turn": turn.turn,
            "omitted": true,
            "total_bytes": topic_bytes,
            "read_via": "detail_field",
            "field": "team.topic",
        }))
    }
}

fn page_turn(meta: Value) -> Value {
    let number = turn_number(&meta).unwrap_or_default();
    json!({
        "meta": meta,
        "detail_fields": {
            "plan": format!("team.turn.{number}.plan"),
            "result_pattern": format!("team.turn.{number}.sub.<sub_turn>.result.<member>"),
            "summary_pattern": format!("team.turn.{number}.sub.<sub_turn>.summary"),
        },
        "turn": number,
    })
}

fn turn_number(value: &Value) -> Option<u64> {
    value["turn"].as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TopicTurnMeta, TOPIC_FINISHED};
    use std::io::Write;

    #[test]
    fn streamed_field_bounds_and_decodes_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("topic.json");
        let mut file = File::create(&path).unwrap();
        write!(
            file,
            "{{\"requirement\":\"a\\n{}\"}}",
            "x".repeat(INLINE_BYTES as usize)
        )
        .unwrap();
        let value = top_level_text(&path, "requirement", INLINE_BYTES).unwrap();
        assert_eq!(value["omitted"], true);
        assert!(value["total_bytes"].as_u64().unwrap() > INLINE_BYTES);
    }

    #[test]
    fn large_topic_projects_first_page_without_retaining_large_fields() {
        let dir = tempfile::tempdir().unwrap();
        let topic = TopicMeta {
            topic_id: "team-projection".into(),
            team_name: "verify".into(),
            title: "projection".into(),
            requirement: "界".repeat(30_000),
            status: TOPIC_FINISHED.into(),
            finish_reason: Some("complete".into()),
            created_at: 1,
            finished_at: Some(2),
            captain: MemberRef {
                node_id: "captain".into(),
                name: "captain".into(),
            },
            members: vec![MemberRef {
                node_id: "captain".into(),
                name: "captain".into(),
            }],
            turns: (1..=51)
                .map(|turn| TopicTurnMeta {
                    turn,
                    question: format!("question {turn}"),
                    participants: vec!["captain".into()],
                    aligned: true,
                    sub_turns: 1,
                })
                .collect(),
            final_summary: Some("结".repeat(30_000)),
        };
        super::super::save_topic(dir.path(), &topic).unwrap();
        let summary = topic_summary(dir.path(), "verify", "team-projection").unwrap();
        assert_eq!(summary["requirement"]["omitted"], true);
        assert_eq!(summary["final_summary"]["omitted"], true);
        assert_eq!(summary["turns"].as_array().unwrap().len(), 50);
        assert_eq!(summary["turns_page"]["next_turn"], 50);
        let page = topic_turns_page(dir.path(), "verify", "team-projection", 50, 50).unwrap();
        assert_eq!(page.turns.len(), 1);
        assert_eq!(page.turns[0]["turn"], 51);
        assert!(page.next_turn.is_none());
    }
}
