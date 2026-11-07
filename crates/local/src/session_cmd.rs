//! CLI dispatch for `config`, `models`, and `session` subcommands.
//!
//! `config show` / `models` are pure P1 (resolved-config display).
//! `session list|show|delete` reads the rdb session store — shared with P3
//! resume.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use opencoder_core::Config;
use opencoder_store::{
    export_bundle, import_bundle, read_bundle, write_bundle, RdbStore, SessionFilter, SessionMeta,
    Store,
};

use crate::{Cli, ConfigSub, SessionSub};

pub async fn config_dispatch(cli: &Cli, sub: &Option<ConfigSub>) -> Result<()> {
    match sub {
        Some(ConfigSub::Show) | None => {
            let workdir = current_workdir(cli)?;
            let cfg = Config::load(&workdir)?;
            println!("{}", config_show_json(&cfg)?);
            Ok(())
        }
        Some(ConfigSub::Set { model }) => {
            let workdir = current_workdir(cli)?;
            let patch = serde_json::json!({ "model": model });
            let path =
                Config::save(&workdir, &patch).context("failed to persist model to config")?;
            let cfg = Config::load(&workdir)?;
            println!(
                "default model \u{2192} {} (provider: {}, id: {})",
                cfg.model,
                cfg.provider_id(),
                cfg.model_id()
            );
            println!("saved \u{2192} {}", path.display());
            Ok(())
        }
    }
}

pub async fn models_dispatch(cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let cfg = Config::load(&workdir)?;
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
    // List all named providers from the `providers` registry.
    if cfg.providers.is_empty() {
        s.push_str("providers    : <none configured — using legacy `provider` field>\n");
    } else {
        let active = cfg.provider_id();
        let mut names: Vec<&String> = cfg.providers.keys().collect();
        names.sort();
        for name in names {
            let p = &cfg.providers[name];
            let mark = if name == active { "*" } else { " " };
            let model_id = p.model.as_deref().unwrap_or("<no default model>");
            s.push_str(&format!(
                "provider {mark}   : {} | {} | {}\n",
                name, p.base_url, model_id
            ));
        }
    }
    s
}

pub async fn session_dispatch(sub: &SessionSub, cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    // The legacy migration owns the store-open sequence (backup → import →
    // marker) and must not run through `open_store`, which both opens the
    // destination rdb store and runs the startup migration check itself.
    if let SessionSub::MigrateLegacy {
        from,
        backup,
        dry_run,
    } = sub
    {
        return migrate_legacy_dispatch(&workdir, from.clone(), backup.clone(), *dry_run).await;
    }
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
            // Match the JSON path: a missing session is an error, not an
            // empty (silent) listing.
            if store.get_session(id).await?.is_none() {
                anyhow::bail!("session not found: {id}");
            }
            for m in store.load_messages(id).await? {
                println!("{}", show_message_line(&m));
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
        SessionSub::MigrateLegacy { .. } => {
            unreachable!("migrate-legacy is dispatched before the store is opened")
        }
    }
}

/// `opencoder session migrate-legacy`: resolve the fixed destination from the
/// workdir's data dir, honor `--from`/`--backup` overrides for the source and
/// backup file, then print the migration report.
/// Resolve migration options: defaults live in the active workdir's data
/// dir; an explicit `--from` outside it re-bases source, derived backup AND
/// destination next to that source, so marker, backup and imported data stay
/// in one place instead of leaking sessions into the current workdir's store.
pub(crate) fn resolve_migrate_opts(
    data_dir: &Path,
    from: Option<PathBuf>,
    backup: Option<PathBuf>,
    dry_run: bool,
) -> crate::legacy::MigrateOptions {
    let mut opts = crate::legacy::MigrateOptions::defaults(data_dir);
    if let Some(source) = &from {
        if *source != opts.source {
            opts.destination = source
                .parent()
                .map(|dir| dir.join("opencoder.db.rdb"))
                .map(PathBuf::from)
                .unwrap_or_else(|| opts.destination.clone());
        }
        opts.backup = crate::legacy::default_backup(source);
        opts.source = source.clone();
    }
    if let Some(backup) = &backup {
        opts.backup = backup.clone();
    }
    opts.dry_run = dry_run;
    opts
}

async fn migrate_legacy_dispatch(
    workdir: &Path,
    from: Option<PathBuf>,
    backup: Option<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let data_dir = opencoder_core::data_dir_for(workdir);
    let opts = resolve_migrate_opts(&data_dir, from, backup, dry_run);
    let report = crate::legacy::run(&opts).await?;
    print!("{}", crate::legacy::render_report(&report, &opts));
    Ok(())
}

/// Build the full session JSON value: meta (incl. compaction summary) + all
/// message blocks (Text/Reasoning/ToolUse/ToolResult) + subagent task records
/// (status/result/ok). Extracted from `show_session_json` so the shape is
/// unit-testable without capturing stdout.
pub(crate) async fn build_session_json(store: &RdbStore, id: &str) -> Result<serde_json::Value> {
    let meta = store
        .get_session(id)
        .await?
        .ok_or_else(|| anyhow!("session not found: {id}"))?;
    // A clear-context boundary (`/act_clear_context`) persists internal
    // markers in handoff_plan so resume can rebuild the fresh-start marker
    // or the last-say seed message. The blank sentinel is redacted entirely;
    // a seed marker is stripped to its preserved text. Raw markers must
    // never be output (handoff_seq still records that a boundary exists).
    let meta = SessionMeta {
        handoff_plan: meta.handoff_plan.and_then(|p| {
            if opencoder_session::is_clear_context_seed(&p) {
                Some(opencoder_session::clear_seed_text(&p).to_string())
            } else {
                (!opencoder_session::is_clear_context_handoff(&p)).then_some(p)
            }
        }),
        ..meta
    };
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
async fn show_session_json(store: &RdbStore, id: &str) -> Result<()> {
    let body = build_session_json(store, id).await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

fn current_workdir(cli: &Cli) -> Result<PathBuf> {
    if let Some(w) = &cli.workdir {
        return Ok(w.clone());
    }
    std::env::current_dir().context("get current dir")
}

/// Open the session store for `workdir`'s hashed data dir: the shared
/// CLI/TUI boot sequence of [`open_store_at`].
pub async fn open_store(workdir: &Path) -> Result<RdbStore> {
    open_store_at(&opencoder_core::data_dir_for(workdir)).await
}

/// Data-dir core of [`open_store`]: best-effort dir creation, legacy-SQLite
/// migration check, then open `session_store_rdb_dir(&data_dir)`. Shared by
/// the CLI and the TUI boot path so the sequence cannot drift — the TUI once
/// opened the legacy plain-file stem directly and failed with
/// `File exists (os error 17)` when an old build had left
/// `<data_dir>/opencoder.db` behind.
pub async fn open_store_at(data_dir: &Path) -> Result<RdbStore> {
    tokio::fs::create_dir_all(data_dir).await.ok();
    // Migrate a legacy SQLite store (if any) BEFORE opening the rdb store so
    // migrated sessions are visible in the same process. A first run without
    // a legacy file is a no-op here and opens normally.
    if let Some(report) = crate::legacy::ensure_migrated(data_dir).await? {
        tracing::info!(
            source = ?report.marker,
            roots = report.roots_imported,
            skipped = report.roots_skipped,
            "migrated legacy session store into the rdb session store"
        );
    }
    RdbStore::open(opencoder_core::session_store_rdb_dir(data_dir)).await
}

/// One transcript line for `session show`: role tag plus the message text.
/// Echo contract: prefer the verbatim `display` (raw user input, `$skill`
/// tokens included); fall back to the recorded blocks for legacy rows.
fn show_message_line(m: &opencoder_core::Message) -> String {
    let text = m.display.clone().unwrap_or_else(|| m.text());
    format!("[{:?}] {}", m.role, text)
}

/// `config show` body: the serialized config with every `api_key` masked
/// (first 4 chars + `***`). Pure (value → [`redact_json`] → pretty string) so
/// the "stdout never carries a full key" contract is unit-testable without
/// spawning the binary.
pub(crate) fn config_show_json(cfg: &Config) -> Result<String> {
    let value = serde_json::to_value(cfg).context("serialize config")?;
    let redacted = opencoder_core::config::redact::redact_json(&value);
    serde_json::to_string_pretty(&redacted).context("serialize config")
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::models_summary;
    use opencoder_core::{Config, ContentBlock, Message, Role};

    /// Echo contract for the `session show` text view: the verbatim display
    /// text wins; a legacy row without `display` falls back to `text()`.
    #[test]
    fn show_message_line_prefers_display_then_blocks() {
        let mut m = Message::user("m1", " fix the bug");
        m.display = Some("$review fix the bug".into());
        assert_eq!(super::show_message_line(&m), "[User] $review fix the bug");

        let legacy = Message::user("m2", "legacy prompt");
        assert_eq!(super::show_message_line(&legacy), "[User] legacy prompt");

        let mut tool = Message {
            provider_state: None,
            id: "m3".into(),
            role: Role::Assistant,
            blocks: vec![ContentBlock::text("done")],
            model: None,
            agent: None,
            usage: Default::default(),
            created_at: 0,
            synthetic: false,
            display: None,
        };
        tool.display = Some("kept".into());
        assert_eq!(super::show_message_line(&tool), "[Assistant] kept");
    }

    #[test]
    fn config_show_json_masks_api_keys() {
        use std::collections::HashMap;

        let mut providers = HashMap::new();
        providers.insert(
            "zhipuai".to_string(),
            opencoder_core::ProviderConfig {
                protocol: "chat_completions".into(),
                base_url: "https://api.example/v1".into(),
                api_key: Some("sk-test-1234567890abcdef".into()),
                model: None,
                headers: Vec::new(),
            },
        );
        providers.insert(
            "short".to_string(),
            opencoder_core::ProviderConfig {
                protocol: "chat_completions".into(),
                base_url: "https://api.example/v1".into(),
                api_key: Some("abcd".into()),
                model: None,
                headers: Vec::new(),
            },
        );
        let cfg = Config {
            providers,
            ..Config::default()
        };
        let out = super::config_show_json(&cfg).unwrap();
        assert!(
            out.contains("\"sk-t***\""),
            "long key must be masked to first 4 chars + ***:\n{out}"
        );
        assert!(
            out.contains("\"***\""),
            "short key (<=4 chars) must be fully masked:\n{out}"
        );
        assert!(
            !out.contains("sk-test-1234567890abcdef"),
            "full api_key values must never appear:\n{out}"
        );
    }

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

    #[test]
    fn models_summary_lists_named_providers() {
        use opencoder_core::ProviderConfig;
        use std::collections::HashMap;

        let mut providers = HashMap::new();
        providers.insert(
            "deepseek".to_string(),
            ProviderConfig {
                protocol: "chat_completions".into(),
                base_url: "https://api.deepseek.com/v1".to_string(),
                api_key: Some("sk-dk".to_string()),
                model: Some("deepseek-chat".to_string()),
                headers: Vec::new(),
            },
        );
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                protocol: "chat_completions".into(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key: None,
                model: Some("gpt-4o".to_string()),
                headers: Vec::new(),
            },
        );
        let cfg = Config {
            model: "deepseek/deepseek-chat".to_string(),
            providers,
            ..Default::default()
        };
        let s = models_summary(&cfg);
        // Both providers listed with their base_url and model.
        assert!(
            s.contains("provider *   : deepseek | https://api.deepseek.com/v1 | deepseek-chat"),
            "active provider (deepseek) must be marked with *, got:\n{s}"
        );
        assert!(
            s.contains("provider     : openai | https://api.openai.com/v1 | gpt-4o"),
            "inactive provider (openai) must be listed without marker, got:\n{s}"
        );
    }

    #[test]
    fn models_summary_shows_none_when_no_providers() {
        let cfg = Config::default();
        let s = models_summary(&cfg);
        assert!(
            s.contains("providers    : <none configured"),
            "empty providers must show the 'none configured' line, got:\n{s}"
        );
    }

    #[tokio::test]
    async fn build_session_json_emits_meta_messages_and_subagent_tasks() {
        use super::build_session_json;
        use opencoder_core::{ContentBlock, Message, Role};
        use opencoder_store::{RdbStore, SessionMeta, Store};

        let store = RdbStore::open_memory().await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "s1".into(),
                title: Some("t".into()),
                agent: Some("act".into()),
                model: Some("m".into()),

                autopilot_mode: None,
                workdir_hash: None,
                created_at: 0,
                updated_at: 0,
                summary: None,
                summary_seq: None,
                summary_images: vec![],
                handoff_seq: None,
                handoff_plan: None,
                skill: None,
                task_type: None,
                requirement: None,
                kind: None,
            })
            .await
            .unwrap();
        let msg = Message {
            provider_state: None,
            display: None,
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
    async fn build_session_json_redacts_clear_context_sentinel() {
        use super::build_session_json;
        use opencoder_store::{RdbStore, SessionMeta, Store};

        let store = RdbStore::open_memory().await.unwrap();
        store
            .create_session(&SessionMeta {
                id: "redact-s1".into(),
                agent: Some("act".into()),
                model: Some("m".into()),

                autopilot_mode: None,
                handoff_seq: Some(1),
                // Exactly what control_cmd::ClearContext persists as the
                // resume-reconstruction boundary marker.
                handoff_plan: Some("<<OPENCODER_CLEAR_CONTEXT_MARKER>>".into()),
                created_at: 0,
                updated_at: 0,
                ..Default::default()
            })
            .await
            .unwrap();

        let body = build_session_json(&store, "redact-s1").await.unwrap();
        let raw = serde_json::to_string(&body).unwrap();
        assert!(
            !raw.contains("<<OPENCODER_CLEAR_CONTEXT_MARKER>>"),
            "sentinel must never be output, got: {raw}"
        );
        assert_eq!(body["meta"]["handoff_seq"], 1, "boundary still visible");
        assert!(
            body["meta"].get("handoff_plan").is_none(),
            "sentinel redacted to None so the marker is never printed"
        );
    }

    #[tokio::test]
    async fn build_session_json_errors_on_missing_session() {
        use super::build_session_json;
        use opencoder_store::RdbStore;

        let store = RdbStore::open_memory().await.unwrap();
        let err = build_session_json(&store, "does-not-exist").await;
        assert!(err.is_err(), "missing session must error, not empty output");
    }

    #[tokio::test]
    async fn config_set_persists_model_to_disk() {
        use crate::{Cli, Command};
        use clap::Parser;

        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        // Pre-create opencoder.json with an editable key so save_target
        // resolves to this project-local file (never touches ~/.opencoder).
        std::fs::write(workdir.join("opencoder.json"), r#"{"model":"old-model"}"#).unwrap();

        let cli = Cli::parse_from([
            "opencoder",
            "--workdir",
            workdir.to_str().unwrap(),
            "config",
            "set",
            "openai/gpt-4o",
        ]);
        if let Some(Command::Config { sub }) = &cli.command {
            super::config_dispatch(&cli, sub).await.unwrap();
        } else {
            panic!("expected Config command");
        }

        // Config file updated with the new model.
        let raw = std::fs::read_to_string(workdir.join("opencoder.json")).unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(cfg["model"], "openai/gpt-4o");
    }

    #[test]
    fn migrate_opts_default_to_the_active_data_dir() {
        let dir = Path::new("/data/hashdir");
        let opts = resolve_migrate_opts(dir, None, None, true);
        assert_eq!(opts.source, dir.join("opencoder.db"));
        assert_eq!(opts.destination, dir.join("opencoder.db.rdb"));
        assert!(opts.dry_run);
    }

    #[test]
    fn migrate_opts_rebase_destination_next_to_an_external_source() {
        let opts = resolve_migrate_opts(
            Path::new("/data/hashdir"),
            Some(PathBuf::from("/data/otherdir/opencoder.db")),
            None,
            false,
        );
        assert_eq!(opts.source, Path::new("/data/otherdir/opencoder.db"));
        assert_eq!(
            opts.destination,
            Path::new("/data/otherdir/opencoder.db.rdb")
        );
        assert_eq!(
            opts.backup,
            Path::new("/data/otherdir/opencoder.db.pre-rdb.bak")
        );
    }

    #[test]
    fn migrate_opts_keep_default_destination_for_explicit_default_source() {
        let dir = Path::new("/data/hashdir");
        let opts = resolve_migrate_opts(
            dir,
            Some(dir.join("opencoder.db")),
            Some(PathBuf::from("/keep.bak")),
            false,
        );
        assert_eq!(opts.destination, dir.join("opencoder.db.rdb"));
        assert_eq!(opts.backup, Path::new("/keep.bak"));
    }
}
