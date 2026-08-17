//! Transport layer for MCP JSON-RPC communication.
//!
//! Two concrete transports:
//! - [`StdioTransport`]: spawns a child process, writes to stdin, reads stdout.
//! - [`MockTransport`]: in-memory channel pair for deterministic unit tests.
//!
//! SSE transport is planned (P1) and returns `Err` until implemented.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

/// Abstracts the bidirectional pipe used to exchange JSON-RPC messages.
///
/// `send` writes a complete JSON-RPC message (no trailing newline needed —
/// the implementation appends one).  `recv` blocks until the next complete
/// message arrives and returns its raw JSON string.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, msg: &str) -> Result<()>;
    async fn recv(&self) -> Result<String>;
    async fn close(&self);
}

// ---------------------------------------------------------------------------
// Stdio
// ---------------------------------------------------------------------------

/// Spawns a child process and communicates over its stdin/stdout pipes.
pub struct StdioTransport {
    stdin: Mutex<tokio::process::ChildStdin>,
    rx: Mutex<mpsc::Receiver<String>>,
    child: Mutex<Option<tokio::process::Child>>,
}

impl StdioTransport {
    /// Spawn the process described by `command`, `args`, `env`.
    ///
    /// Falls back to `sh -c <command>` when `command` looks like it contains
    /// shell metacharacters (e.g. `npx -y @mcp/server`).
    pub fn spawn(command: &str, args: &[String], env: &HashMap<String, String>) -> Result<Self> {
        let (program, owned_args) = if command.contains(' ') && args.is_empty() {
            // Shell form: run via `sh -c`.
            (
                "sh".to_string(),
                vec!["-c".to_string(), command.to_string()],
            )
        } else {
            (command.to_string(), args.to_vec())
        };

        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&owned_args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| anyhow!("failed to spawn MCP server `{program}`: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP child stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP child stdout not piped"))?;

        let (tx, rx) = mpsc::channel::<String>(128);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        if tx.send(line).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            stdin: Mutex::new(stdin),
            rx: Mutex::new(rx),
            child: Mutex::new(Some(child)),
        })
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&self, msg: &str) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(msg.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn recv(&self) -> Result<String> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow!("MCP transport closed"))
    }

    async fn close(&self) {
        // Kill the child process (kill_on_drop also fires on drop, but
        // explicit kill ensures the reader task unblocks promptly).
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }
}

/// Build a transport from a server config.  Returns `Err` for SSE (not yet
/// implemented) and for configs with no transport.
pub fn build_from_config(
    cfg: &opencoder_core::config::McpServerConfig,
) -> Result<Box<dyn McpTransport>> {
    if let Some(cmd) = &cfg.command {
        Ok(Box::new(StdioTransport::spawn(cmd, &cfg.args, &cfg.env)?))
    } else if cfg.url.is_some() {
        Err(anyhow!("SSE transport not yet implemented"))
    } else {
        Err(anyhow!("MCP server has neither `command` nor `url`"))
    }
}

// ---------------------------------------------------------------------------
// Mock (for unit tests)
// ---------------------------------------------------------------------------

/// In-memory transport backed by two unbounded channels — one per direction.
/// Both ends are `Clone`-able so test setups can pre-load responses.
#[derive(Clone)]
pub struct MockTransport {
    pub tx: mpsc::UnboundedSender<String>,
    pub rx: Arc<Mutex<mpsc::UnboundedReceiver<String>>>,
}

impl MockTransport {
    pub fn pair() -> (MockTransport, MockTransport) {
        let (tx_a, rx_a) = mpsc::unbounded_channel();
        let (tx_b, rx_b) = mpsc::unbounded_channel();
        let a = MockTransport {
            tx: tx_a,
            rx: Arc::new(Mutex::new(rx_b)),
        };
        let b = MockTransport {
            tx: tx_b,
            rx: Arc::new(Mutex::new(rx_a)),
        };
        (a, b)
    }

    /// Send a raw line to the peer's `recv`.
    pub fn send_raw(&self, line: impl Into<String>) {
        let _ = self.tx.send(line.into());
    }
}

#[async_trait]
impl McpTransport for MockTransport {
    async fn send(&self, msg: &str) -> Result<()> {
        self.tx
            .send(msg.to_string())
            .map_err(|_| anyhow!("mock transport closed"))
    }

    async fn recv(&self) -> Result<String> {
        self.rx
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| anyhow!("mock transport closed"))
    }

    async fn close(&self) {
        self.tx.send(String::new()).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_transport_roundtrip() {
        let (a, b) = MockTransport::pair();
        a.send("hello").await.unwrap();
        let msg = b.recv().await.unwrap();
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn mock_transport_bidirectional() {
        let (a, b) = MockTransport::pair();
        a.send("ping").await.unwrap();
        let got = b.recv().await.unwrap();
        assert_eq!(got, "ping");
        b.send("pong").await.unwrap();
        let got = a.recv().await.unwrap();
        assert_eq!(got, "pong");
    }
}
