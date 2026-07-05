//! CLI dispatch for `config`, `models`, and `session` subcommands.
//!
//! `config show` / `models` are pure P1 (resolved-config display).
//! `session list|show|delete` reads the libsql store — shared with P3 resume.

use std::path::PathBuf;

use anyhow::{Context, Result};

use opencode_core::Config;
use opencode_store::{LibsqlStore, SessionFilter, Store};

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

/// Render the `opencode models` summary as a string. Extracted from
/// `models_dispatch` so the reasoning_effort display path is unit-testable
/// without spawning the binary or a live model.
pub(crate) fn models_summary(cfg: &Config) -> String {
    let mut s = String::new();
    s.push_str(&format!("primary      : {}  (provider: {}, id: {})\n", cfg.model, cfg.provider_id(), cfg.model_id()));
    match &cfg.small_model {
        Some(m) => s.push_str(&format!("small_model  : {m}\n")),
        None => s.push_str("small_model  : <unset, falls back to primary>\n"),
    }
    match &cfg.reasoning_effort {
        Some(e) => s.push_str(&format!("thinking     : {e}  (reasoning_effort)\n")),
        None => s.push_str("thinking     : <unset, provider default>\n"),
    }
    s.push_str(&format!("context_limit: {}\n", cfg.context_limit()));
    s.push_str(&format!(
        "compaction   : auto={} threshold={} reserved={} tail_turns={} prune={}\n",
        cfg.compaction.auto,
        cfg.compaction.context_threshold,
        cfg.compaction.reserved,
        cfg.compaction.tail_turns,
        cfg.compaction.prune,
    ));
    s
}

pub async fn session_dispatch(sub: &SessionSub, cli: &Cli) -> Result<()> {
    let workdir = current_workdir(cli)?;
    let store = open_store(&workdir).await?;
    match sub {
        SessionSub::List => {
            let items = store.list_sessions(&SessionFilter { limit: 50, ..Default::default() }).await?;
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
        SessionSub::Show { id } => {
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
    }
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
    LibsqlStore::open(data_dir.join("opencode.db")).await
}

fn data_dir_for(workdir: &PathBuf) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    workdir.hash(&mut h);
    let digest = h.finish();
    let mut base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("opencode");
    base.push(format!("{digest:x}"));
    base
}

#[cfg(test)]
mod tests {
    use super::models_summary;
    use opencode_core::Config;

    #[test]
    fn models_summary_shows_reasoning_effort_when_set() {
        let cfg = Config { reasoning_effort: Some("medium".into()), ..Default::default() };
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
}
