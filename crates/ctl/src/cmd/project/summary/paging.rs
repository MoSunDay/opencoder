//! Native pagination, including full transcript chunks and oversized fields.
use super::read;
use crate::{
    ctx::Ctx,
    http::{self, RequestPlan},
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

async fn pages(
    ctx: &Ctx,
    path: &str,
    rows: &str,
    cursor: &str,
    next: &str,
    snapshot_events: bool,
) -> Result<Value> {
    let mut result = vec![];
    let mut at = None;
    let mut head = None;
    loop {
        let page = read(
            ctx,
            RequestPlan::get(path).with_opt(cursor, at.map(|at: i64| at.to_string())),
        )
        .await?;
        let items = page[rows].as_array().context("missing page rows")?;
        if snapshot_events && head.is_none() {
            head = page["head_seq"].as_i64();
        }
        result.extend(
            items
                .iter()
                .filter(|row| {
                    head.is_none_or(|head| row["seq"].as_i64().is_none_or(|seq| seq <= head))
                })
                .cloned(),
        );
        if page["more"] == false {
            break;
        }
        anyhow::ensure!(page["more"] == true, "missing pagination completion flag");
        let following = if rows == "events" {
            items.last().and_then(|row| row["seq"].as_i64())
        } else {
            page[next].as_i64()
        }
        .context("missing next cursor")?;
        anyhow::ensure!(
            at.is_none_or(|at| following > at),
            "pagination cursor did not advance"
        );
        if head.is_some_and(|head| following >= head) {
            break;
        }
        at = Some(following);
    }
    Ok(json!(result))
}

async fn messages(ctx: &Ctx, base: &str) -> Result<Value> {
    let mut cursor = (0, 0);
    let mut result = BTreeMap::<i64, (Value, Vec<u8>)>::new();
    loop {
        let page = read(
            ctx,
            RequestPlan::get(format!("{base}/messages"))
                .with("seq", cursor.0.to_string())
                .with("offset", cursor.1.to_string()),
        )
        .await?;
        for chunk in page["chunks"]
            .as_array()
            .context("missing message chunks")?
        {
            let seq = chunk["seq"].as_i64().context("missing message sequence")?;
            let entry = result.entry(seq).or_insert_with(|| {
                (
                    json!({"seq":seq,"role":chunk["role"],"created_at":chunk["created_at"]}),
                    vec![],
                )
            });
            anyhow::ensure!(
                chunk["offset"].as_u64() == Some(entry.1.len() as u64),
                "non-contiguous message chunks"
            );
            entry.1.extend(
                STANDARD.decode(
                    chunk["bytes_b64"]
                        .as_str()
                        .context("missing message bytes")?,
                )?,
            );
        }
        if page["more"] == false {
            break;
        }
        anyhow::ensure!(page["more"] == true, "missing message completion flag");
        let next = (&page["next_cursor"]["seq"], &page["next_cursor"]["offset"]);
        let next = (
            next.0.as_i64().context("missing message cursor")?,
            next.1.as_u64().context("missing message offset")?,
        );
        anyhow::ensure!(next > cursor, "message cursor did not advance");
        cursor = next;
    }
    let mut rows = vec![];
    for (_, (mut row, bytes)) in result {
        let text = String::from_utf8(bytes)?;
        row["content"] = serde_json::from_str(&text).unwrap_or(json!(text));
        rows.push(row);
    }
    Ok(json!(rows))
}

fn omitted(value: &Value, fields: &mut HashSet<String>) {
    if value["omitted"] == true && value["read_via"] == "detail_field" {
        if let Some(field) = value["field"].as_str() {
            fields.insert(field.into());
        }
    }
    match value {
        Value::Object(map) => {
            for child in map.values() {
                omitted(child, fields);
            }
        }
        Value::Array(rows) => {
            for child in rows {
                omitted(child, fields);
            }
        }
        _ => {}
    }
}

async fn chunks(ctx: &Ctx, plan: RequestPlan) -> Result<Value> {
    let mut bytes = vec![];
    loop {
        let page = read(ctx, plan.clone().with("offset", bytes.len().to_string())).await?;
        anyhow::ensure!(
            page["offset"].as_u64() == Some(bytes.len() as u64),
            "non-contiguous field chunks"
        );
        let chunk = STANDARD.decode(page["bytes_b64"].as_str().context("missing field bytes")?)?;
        anyhow::ensure!(
            !chunk.is_empty() || page["eof"] == true,
            "field cursor did not advance"
        );
        bytes.extend(chunk);
        if page["eof"] == true {
            break;
        }
        anyhow::ensure!(
            page["next_offset"].as_u64() == Some(bytes.len() as u64),
            "invalid field cursor"
        );
    }
    let text = String::from_utf8(bytes)?;
    Ok(serde_json::from_str(&text).unwrap_or(json!(text)))
}

fn save(result: Result<Value>, source: &str, issues: &mut Vec<Value>) -> Value {
    match result {
        Ok(value) => value,
        Err(error) => {
            issues.push(json!({"source":source,"error":error.to_string()}));
            Value::Null
        }
    }
}

pub(in crate::cmd::project) async fn execution(
    ctx: &Ctx,
    id: &str,
    kind: &str,
    issues: &mut Vec<Value>,
) -> Value {
    let base = format!("/api/executions/{}", http::urlencode(id));
    let detail = save(read(ctx, RequestPlan::get(&base)).await, &base, issues);
    let resolved_kind = detail["execution"]["kind"]
        .as_str()
        .or_else(|| detail["request"]["kind"].as_str())
        .unwrap_or(kind)
        .to_owned();
    let kind = resolved_kind.as_str();
    let result_path = format!("{base}/result");
    let conclusion = save(
        read(ctx, RequestPlan::get(&result_path)).await,
        &result_path,
        issues,
    );
    let path = format!("{base}/events-page");
    let mut events = save(
        // Brain reports the last sequence of this page, not a replay watermark.
        // Its `more` flag must drive pagination even when head_seq is reached.
        pages(ctx, &path, "events", "after", "", kind != "brain").await,
        &path,
        issues,
    );
    for event in events.as_array_mut().into_iter().flatten() {
        let data = event["data"]
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or_else(|| event["data"].clone());
        if data["omitted"] == true && data["read_via"] == "event_payload" {
            if let Some(seq) = event["seq"].as_i64() {
                let path = format!("{base}/events/{seq}/payload");
                event["data"] = save(chunks(ctx, RequestPlan::get(&path)).await, &path, issues);
            }
        }
    }
    let mut evidence = json!({"kind":kind,"detail":detail,"conclusion":conclusion,"events":events});
    if matches!(kind, "agent" | "operator") {
        evidence["messages"] = save(
            messages(ctx, &base).await,
            &format!("{base}/messages"),
            issues,
        );
    }
    if matches!(kind, "team" | "todos") {
        let (resource, rows, cursor, next) = if kind == "team" {
            ("team-turns", "turns", "after_turn", "next_turn")
        } else {
            ("todo-items", "items", "after_ordinal", "next_ordinal")
        };
        let path = format!("{base}/{resource}");
        evidence[rows] = save(
            pages(ctx, &path, rows, cursor, next, false).await,
            &path,
            issues,
        );
    }
    let mut fields = HashSet::new();
    omitted(&evidence, &mut fields);
    let mut full_fields = serde_json::Map::new();
    for name in fields {
        let source = format!("{base}/detail-field?field={name}");
        full_fields.insert(
            name.clone(),
            save(
                chunks(
                    ctx,
                    RequestPlan::get(format!("{base}/detail-field")).with("field", &name),
                )
                .await,
                &source,
                issues,
            ),
        );
    }
    evidence["full_fields"] = json!(full_fields);
    if evidence["conclusion"]["truncated"] == true || evidence["conclusion"]["omitted"] == true {
        issues.push(json!({"source":result_path,"error":"结论接口省略了部分内容，请结合完整消息、过程及 full_fields 核对"}));
    }
    evidence
}
