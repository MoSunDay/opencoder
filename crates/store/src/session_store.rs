use std::path::{Path, PathBuf};

use anyhow::Result;
use opencoder_core::Message;

pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(root: PathBuf) -> Self {
        SessionStore { root }
    }

    pub fn jsonl(&self, session_id: &str) -> crate::JsonlStore {
        crate::JsonlStore::new(&self.root.join("sessions"), session_id)
    }

    pub async fn list(&self) -> Result<Vec<String>> {
        let dir = self.root.join("sessions");
        let mut out = Vec::new();
        if let Ok(mut rd) = tokio::fs::read_dir(&dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(id) = name.strip_suffix(".jsonl") {
                    out.push(id.to_string());
                }
            }
        }
        out.sort();
        out.reverse();
        Ok(out)
    }

    pub async fn load(&self, session_id: &str) -> Result<Vec<Message>> {
        self.jsonl(session_id).load().await
    }

    pub async fn append(&self, session_id: &str, msg: &Message) -> Result<()> {
        self.jsonl(session_id).append(msg).await
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
