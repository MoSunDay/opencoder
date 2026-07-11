use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use opencode_core::Message;
use tokio::io::AsyncWriteExt;

pub struct JsonlStore {
    path: PathBuf,
}

impl JsonlStore {
    pub fn new(dir: &Path, session_id: &str) -> Self {
        let path = dir.join(format!("{session_id}.jsonl"));
        JsonlStore { path }
    }

    pub async fn append(&self, msg: &Message) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let line = serde_json::to_string(msg).context("serialize message")?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .context("open jsonl")?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }

    pub async fn append_many(&self, msgs: &[Message]) -> Result<()> {
        for m in msgs {
            self.append(m).await?;
        }
        Ok(())
    }

    pub async fn load(&self) -> Result<Vec<Message>> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => {
                let mut out = Vec::new();
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Message>(line) {
                        Ok(m) => out.push(m),
                        Err(e) => {
                            tracing::warn!(error = %e, "skipping malformed jsonl line");
                        }
                    }
                }
                Ok(out)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e).context("read jsonl"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
