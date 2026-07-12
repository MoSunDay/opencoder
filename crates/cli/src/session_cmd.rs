//! CLI dispatch for `config`, `models`, and `session` subcommands.
//!
//! `config show` / `models` are pure P1 (resolved-config display).
//! `session list|show|delete` reads the libsql store — shared with P3 resume.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};

use opencode_core::Config;
use opencode_store::{
    export_bundle, import_bundle, read_bundle, write_bundle, LibsqlStore, SessionFilter, Store,
};

use crate::{Cli, ConfigSub, SessionSub};

pub async fn config_dispatch(cli: &Cli, sub: &Option<ConfigSub>) -> Result<()> {
    match sub {
        Some(ConfigSub::Show) | None => {
            let workdir = current_workdir(cli)?;
            let cfg = Config::load(&workdir)?;
            let json = serde_json::to_string_pretty(&cfg).context("serialize config")?;
            println!("{json}");
            Ok(())
        }
    }
}

pub async fn models_dispatch(cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let mut cfg = Config::load(&workdir)?;
    apply_cli_overrides(cli, &mut cfg);
    print!("{}", models_summary(&cfg));
    Ok(())
}

/// Render the `opencoder models` summary as a string. Extracted from
/// `models_dispatch` so the reasoning_effort display path is unit-testable
/// without spawning the binary or a live model.
pub(crate) fn models_summary(cfg: &Config) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "primary      : {}  (provider: {}, id: {})\n",
        cfg.model,
        cfg.provider_id(),
        cfg.model_id()
    ));
    match &cfg.small_model {
        Some(m) => s.push_str(&format!("small_model  : {m}\n")),
        None => s.push_str("small_model  : <unset, falls back to primary>\n"),
    }
    match &cfg.reasoning_effort {
        Some(e) => s.push_str(&format!("thinking     : {e}  (reasoning_effort)\n")),
        None => s.push_str("thinking     : <unset, provider default>\n"),
    }
    match cfg.interleaved_thinking {
        Some(true) => {
            s.push_str("interleave   : on  (reasoning_content round-trip on tool turns)\n")
        }
        Some(false) => s.push_str("interleave   : off\n"),
        None => s.push_str("interleave   : <unset, defaults on>\n"),
    }
    s.push_str(&format!("context_limit: {}\n", cfg.context_limit()));
    s.push_str(&format!(
        "compaction   : auto={} threshold={} reserved={} tail_turns={}\n",
        cfg.compaction.auto,
        cfg.compaction.context_threshold,
        cfg.compaction.reserved,
        cfg.compaction.tail_turns,
    ));
    s
}

pub async fn session_dispatch(sub: &SessionSub, cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let store = open_store(&workdir).await?;
    match sub {
        SessionSub::List => {
            let items = store
                .list_sessions(&SessionFilter {
                    limit: 50,
                    ..Default::default()
                })
                .await?;
            if items.is_empty() {
                println!("(no sessions for this workdir)");
                return Ok(());
            }
            for it in items {
                let title = it.title.unwrap_or_else(|| "(untitled)".into());
                println!("{}\t{}\t{}", it.id, title, it.preview);
            }
            Ok(())
        }
        SessionSub::Show { id, json } => {
            if *json {
                return show_session_json(&store, id).await;
            }
            for m in store.load_messages(id).await? {
                println!("[{:?}] {}", m.role, m.text());
            }
            Ok(())
        }
        SessionSub::Delete { id } => {
            store.delete_session(id).await?;
            println!("deleted {id}");
            Ok(())
        }
        SessionSub::Export { id, out } => {
            let bundle = export_bundle(&store, id).await?;
            let path = out
                .clone()
                .unwrap_or_else(|| format!("{id}.opencoder").into());
            let mut file = std::fs::File::create(&path)
                .with_context(|| format!("create {}", path.display()))?;
            write_bundle(&bundle, &mut file)?;
            let sub_count = bundle.subagents.len();
            println!("exported {id} ({sub_count} subagents) → {}", path.display());
            Ok(())
        }
        SessionSub::Import { input } => {
            let mut file = std::fs::File::open(input).with_context(|| "open bundle file")?;
            let bundle = read_bundle(&mut file)?;
            let id = import_bundle(&store, &bundle, None).await?;
            println!(
                "imported session {id} ({} messages, {} subagents)",
                bundle.messages.len(),
                bundle.subagents.len()
            );
            println!("continue with: opencoder --session {id}");
            Ok(())
        }
    }
}

/// Build the full session JSON value: meta (incl. compaction summary) + all
/// message blocks (Text/Reasoning/ToolUse/ToolResult) + subagent task records
/// (status/result/ok). Extracted from `show_session_json` so the shape is
/// unit-testable without capturing stdout.
pub(crate) async fn build_session_json(store: &LibsqlStore, id: &str) -> Result<serde_json::Value> {
    let meta = store
        .get_session(id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {id}"))?;
    let messages = store.load_messages(id).await?;
    let subagent_tasks = store.list_subagent_tasks(id).await?;
    Ok(serde_json::json!({
        "meta": meta,
        "messages": messages,
        "subagent_tasks": subagent_tasks,
    }))
}

/// Emit full session state as JSON (see `build_session_json`). Machine-readable
/// surface for deep e2e assertions, decoupled from storage internals.
async fn show_session_json(store: &LibsqlStore, id: &str) -> Result<()> {
    let body = build_session_json(store, id).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

pub(crate) fn apply_cli_overrides(cli: &Cli, cfg: &mut Config) {
    if let Some(m) = &cli.model {
        cfg.model = m.clone();
    }
    if let Some(m) = &cli.small_model {
        cfg.small_model = Some(m.clone());
    }
}

fn current_workdir(cli: &Cli) -> Result<PathBuf> {
    if let Some(w) = &cli.workdir {
        return Ok(w.clone());
    }
    std::env::current_dir().context("get current dir")
}

pub(crate) async fn open_store(workdir: &PathBuf) -> Result<LibsqlStore> {
    let data_dir = data_dir_for(workdir);
    tokio::fs::create_dir_all(&data_dir).await.ok();
    LibsqlStore::open(data_dir.join("opencoder.db")).await
}

fn data_dir_for(workdir: &PathBuf) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workdir.hash(&mut h);
    let digest = h.finish();
    let mut base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("opencoder");
    base.push(format!("{digest:x}"));
    base
}

#[cfg(test)]
mod tests {
    use super::models_summary;
    use opencode_core::Config;

    #[test]
    fn models_summary_shows_reasoning_effort_when_set() {
        let cfg = Config {
            reasoning_effort: Some("medium".into()),
            ..Default::default()
        };
        let s = models_summary(&cfg);
        assert!(
            s.contains("thinking     : medium  (reasoning_effort)"),
            "reasoning_effort line must appear, got:\n{s}"
        );
    }

    #[test]
    fn models_summary_shows_unset_when_absent() {
        let cfg = Config::default();
        let s = models_summary(&cfg);
        assert!(
            s.contains("thinking     : <unset, provider default>"),
            "absent reasoning_effort must render unset marker, got:\n{s}"
        );
    }

    #[test]
    fn models_summary_shows_interleave_on_by_default() {
        let cfg = Config::default();
        let s = models_summary(&cfg);
        assert!(
            s.contains("interleave   : on  (reasoning_content round-trip on tool turns)"),
            "default interleaved_thinking must render on, got:\n{s}"
        );
    }

    #[test]
    fn models_summary_shows_interleave_off() {
        let cfg = Config {
            interleaved_thinking: Some(false),
            ..Default::default()
        };
        let s = models_summary(&cfg);
        assert!(
            s.contains("interleave   : off"),
            "interleaved_thinking=false must render off, got:\n{s}"
        );
    }

    #[tokio::test]
    async fn build_session_json_emits_meta_messages_and_subagent_tasks() {
        use super::build_session_json;
        use opencode_core::{ContentBlock, Message, Role};
        use opencode_store::{LibsqlStore, SessionMeta, Store};

        let store = LibsqlStore::open_memory().await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "s1".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
            })
            .await
            .unwrap();
        let msg = Message {
            id: "m1".into(),
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "hello".into(),
                },
                ContentBlock::ToolUse {
                    id: "tu1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ],
            model: None,
            agent: None,
            usage: Default::default(),
            created_at: 0,
            synthetic: false,
        };
        store.append_message("s1", &msg).await.unwrap();

        let body = build_session_json(&store, "s1").await.unwrap();
        assert_eq!(body["meta"]["id"], "s1", "meta.id must round-trip");
        let messages = body["messages"].as_array().expect("messages is array");
        assert_eq!(messages.len(), 1, "one message persisted");
        // Tool-use block survives — NOT filtered to text (the whole point of --json).
        let blocks = messages[0]["blocks"].as_array().expect("blocks is array");
        assert_eq!(blocks.len(), 2, "both content blocks present");
        assert_eq!(blocks[1]["kind"], "tool_use");
        assert_eq!(blocks[1]["name"], "bash");
        assert_eq!(
            body["subagent_tasks"].as_array().unwrap().len(),
            0,
            "no subagent tasks"
        );
    }

    #[tokio::test]
    async fn build_session_json_errors_on_missing_session() {
        use super::build_session_json;
        use opencode_store::LibsqlStore;

        let store = LibsqlStore::open_memory().await.unwrap();
        let err = build_session_json(&store, "does-not-exist").await;
        assert!(err.is_err(), "missing session must error, not empty output");
    }
}
