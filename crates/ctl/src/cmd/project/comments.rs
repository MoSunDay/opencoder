//! Maintain notes through the same authenticated Server API as the UI.
use crate::http::{urlencode, RequestPlan};
use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::Value;

#[derive(Debug, Subcommand)]
pub enum CommentsCmd {
    List {
        id: String,
        #[arg(long, conflicts_with = "after_seq")]
        before_seq: Option<i64>,
        #[arg(long)]
        after_seq: Option<i64>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Body: {request_id,body_md,capability_id?,execution_id?}; retry with the same request_id.
    Add {
        id: String,
        #[arg(long)]
        json: String,
    },
    /// Replace Markdown only. Body: {body_md,expected_revision} from the latest read.
    Edit {
        id: String,
        seq: i64,
        #[arg(long)]
        json: String,
    },
    /// Delete a comment using its latest revision; retries do not recreate it.
    Delete {
        id: String,
        seq: i64,
        #[arg(long)]
        expected_revision: i64,
    },
}

pub fn plan(command: &CommentsCmd) -> Result<RequestPlan> {
    Ok(match command {
        CommentsCmd::List {
            id,
            before_seq,
            after_seq,
            limit,
        } => {
            anyhow::ensure!(
                before_seq.is_none_or(|v| v > 0)
                    && after_seq.is_none_or(|v| v > 0)
                    && !(before_seq.is_some() && after_seq.is_some())
                    && limit.is_none_or(|v| (1..=100).contains(&v)),
                "invalid comment pagination"
            );
            RequestPlan::get(format!("/api/project/todos/{}/comments", urlencode(id)))
                .with_opt("before_seq", before_seq.map(|v| v.to_string()))
                .with_opt("after_seq", after_seq.map(|v| v.to_string()))
                .with_opt("limit", limit.map(|v| v.to_string()))
        }
        CommentsCmd::Add { id, json } => {
            RequestPlan::post(format!("/api/project/todos/{}/comments", urlencode(id)))
                .with_body(super::required_body(json)?)
        }
        CommentsCmd::Edit { id, seq, json } => {
            anyhow::ensure!(*seq > 0, "invalid comment sequence");
            RequestPlan::patch(format!(
                "/api/project/todos/{}/comments/{seq}",
                urlencode(id)
            ))
            .with_body(super::required_body(json)?)
        }
        CommentsCmd::Delete {
            id,
            seq,
            expected_revision,
        } => {
            anyhow::ensure!(
                *seq > 0 && *expected_revision > 0,
                "invalid comment sequence or revision"
            );
            RequestPlan::delete(format!(
                "/api/project/todos/{}/comments/{seq}",
                urlencode(id)
            ))
            .with("expected_revision", expected_revision.to_string())
        }
    })
}

pub(super) async fn all(ctx: &crate::ctx::Ctx, id: &str) -> Result<Value> {
    let path = format!("/api/project/todos/{}/comments", urlencode(id));
    let mut comments = vec![];
    let mut cursor: Option<i64> = None;
    loop {
        let page = super::summary::read(
            ctx,
            RequestPlan::get(&path)
                .with_opt("before_seq", cursor.map(|v| v.to_string()))
                .with_opt("limit", Some("100".into())),
        )
        .await?;
        let rows = page["comments"].as_array().context("missing comments")?;
        comments.extend(rows.iter().cloned());
        if !page["has_more"]
            .as_bool()
            .context("missing comment pagination state")?
        {
            break;
        }
        let next = rows
            .first()
            .and_then(|row| row["seq"].as_i64())
            .context("missing comment cursor")?;
        anyhow::ensure!(
            cursor.is_none_or(|old| next < old),
            "comment cursor did not advance"
        );
        cursor = Some(next);
    }
    comments.sort_by_key(|row| row["seq"].as_i64());
    Ok(Value::Array(comments))
}
