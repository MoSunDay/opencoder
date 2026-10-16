//! Keep a host account broker alive until the runc step tree has been reaped.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

pub struct Broker {
    child: Child,
    path: PathBuf,
}

impl Broker {
    pub async fn start(
        root: &Path,
        key: &str,
        env: &mut Vec<(String, String)>,
    ) -> Result<Option<Self>> {
        let Some((_, launch)) = env
            .iter()
            .find(|(name, _)| name == "OPENCODER_CODEX_LAUNCH")
        else {
            return Ok(None);
        };
        let relative = Path::new(launch)
            .strip_prefix("/run/opencoder-private/codex")
            .context("Codex launch outside private mount")?;
        let binding = root
            .join("private/codex")
            .join(relative)
            .parent()
            .context("Codex launch parent missing")?
            .join("startup-binding.json");
        let bytes = match std::fs::read(binding) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let startup: super::startup::Managed = serde_json::from_slice(&bytes)?;
        let session = env
            .iter()
            .find(|(name, _)| name == "OPENCODER_STEP_SESSION_ID")
            .context("managed Codex session missing")?
            .1
            .clone();
        let name = format!("{:.32x}.sock", Sha256::digest(key.as_bytes()));
        let path = root.join("private/exec").join(&name);
        let diagnostic =
            std::fs::File::create(root.join("private/exec").join(format!("{key}.broker.log")))?;
        // The broker is deliberately not kill_on_drop: unexpected Runtime loss
        // closes stdin and makes it retain the lease until its container dies.
        let mut child = Command::new(&startup.program)
            .arg("--config")
            .arg(&startup.config)
            .arg("--broker-socket")
            .arg(&path)
            .arg("--broker-pid-file")
            .arg(root.join("private/exec").join(format!("{key}.pid")))
            .env("OPENCODER_STEP_SESSION_ID", session)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(diagnostic)
            .spawn()
            .context("start private host Codex account broker")?;
        for _ in 0..100 {
            if path.exists() {
                env.push((
                    "OPENCODER_CODEX_BROKER".into(),
                    format!("/run/opencoder-private/exec/{name}"),
                ));
                return Ok(Some(Self { child, path }));
            }
            ensure!(
                child.try_wait()?.is_none(),
                "private Codex account broker failed before readiness"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        child.kill().await?;
        anyhow::bail!("private Codex account broker readiness timed out")
    }

    pub async fn finish(mut self, code: i32) -> Result<()> {
        if let Some(mut input) = self.child.stdin.take() {
            input
                .write_all(format!("{{\"exit_code\":{code}}}\n").as_bytes())
                .await?;
            input.shutdown().await?;
        }
        let status = tokio::time::timeout(Duration::from_secs(90), self.child.wait())
            .await
            .context("private Codex broker cleanup timed out; reservation retained")??;
        ensure!(
            status.success(),
            "private Codex broker failed; inspect private receipt"
        );
        ensure!(
            !self.path.exists(),
            "private Codex broker socket was not removed"
        );
        Ok(())
    }
}
