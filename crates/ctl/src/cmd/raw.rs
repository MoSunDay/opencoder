//! Escape hatch: drive any server route verbatim. Guarantees 100% API
//! operability even where no dedicated wrapper exists.

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::cmd::{exec_plan, exec_stream};
use crate::ctx::Ctx;
use crate::http::RequestPlan;

#[derive(Subcommand, Debug)]
pub enum RawCmd {
    /// Send an arbitrary request: METHOD PATH [--json ...] [--query k=v].
    #[command(alias = "send")]
    Call {
        /// HTTP method (GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS).
        method: String,
        /// Absolute API path, e.g. /api/nodes.
        path: String,
        /// Request body: inline JSON or @file with JSON content.
        #[arg(long)]
        json: Option<String>,
        /// Query pair, repeatable: --query k=v.
        #[arg(long = "query")]
        queries: Vec<String>,
        /// Treat the response as an SSE stream and print JSON frames.
        #[arg(long)]
        stream: bool,
    },
}

/// Parse a `--json` value: `@path` reads a file, anything else is inline
/// JSON text. Empty/invalid JSON is an error (never silently send garbage).
pub fn parse_body(raw: Option<&str>) -> Result<Option<serde_json::Value>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("read body file {path}"))?
    } else {
        raw.to_owned()
    };
    serde_json::from_str(&text)
        .map(Some)
        .with_context(|| "parse --json body (expected JSON text or @file)")
}

/// Split `k=v` query pairs; rejects pairs without '='.
pub fn parse_queries(pairs: &[String]) -> Result<Vec<(String, String)>> {
    pairs
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .with_context(|| format!("invalid --query pair {pair:?}: expected k=v"))
        })
        .collect()
}

pub fn plan(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    queries: Vec<(String, String)>,
) -> Result<RequestPlan> {
    let upper = method.to_ascii_uppercase();
    let method = match upper.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => {
            upper.parse::<reqwest::Method>().expect("known method")
        }
        other => anyhow::bail!("unsupported HTTP method {other:?}"),
    };
    let mut plan = RequestPlan {
        method,
        path: path.to_owned(),
        query: Vec::new(),
        body: None,
    };
    for (key, value) in queries {
        plan = plan.with(&key, value);
    }
    Ok(plan.with_opt_body(body))
}

pub async fn run(ctx: &Ctx, cmd: RawCmd) -> Result<i32> {
    let RawCmd::Call {
        method,
        path,
        json,
        queries,
        stream,
    } = cmd;
    let request = plan(
        &method,
        &path,
        parse_body(json.as_deref())?,
        parse_queries(&queries)?,
    )?;
    if stream {
        return exec_stream(ctx, request).await;
    }
    exec_plan(ctx, request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_inline_and_at_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.json");
        std::fs::write(&file, r#"{"a":1}"#).unwrap();
        assert_eq!(
            parse_body(Some(r#"{"b":2}"#)).unwrap(),
            Some(json!({"b": 2}))
        );
        assert_eq!(
            parse_body(Some(format!("@{}", file.display()).as_str())).unwrap(),
            Some(json!({"a": 1}))
        );
        assert_eq!(parse_body(None).unwrap(), None);
        assert!(parse_body(Some("not json")).is_err());
    }

    #[test]
    fn query_pairs_split_and_validate() {
        let pairs = parse_queries(&["a=1".into(), "b=x%20y".into()]).unwrap();
        assert_eq!(
            pairs,
            vec![("a".into(), "1".into()), ("b".into(), "x%20y".into())]
        );
        assert!(parse_queries(&["novalue".into()]).is_err());
    }

    #[test]
    fn plan_assembles_method_path_query_body() {
        let request = plan(
            "post",
            "/api/executions",
            Some(json!({"kind": "agent"})),
            vec![("limit".into(), "5".into())],
        )
        .unwrap();
        assert_eq!(request.method, reqwest::Method::POST);
        assert_eq!(request.path, "/api/executions");
        assert_eq!(request.query, vec![("limit".to_owned(), "5".to_owned())]);
        assert_eq!(request.body, Some(json!({"kind": "agent"})));
        assert!(plan("bogus", "/x", None, vec![]).is_err());
    }
}
