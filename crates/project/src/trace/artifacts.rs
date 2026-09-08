//! Declared deliverables are copied once and addressed by digest, never by user paths.
use super::archive::{write_new, Archive};
use anyhow::{Context, Result};
use opencoder_core::{Tool, ToolContext, ToolOutput};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    sync::Arc,
};

struct RegisterArtifact {
    archive: Archive,
}
#[async_trait::async_trait]
impl Tool for RegisterArtifact {
    fn name(&self) -> &str {
        "project_artifact"
    }
    fn description(&self) -> &str {
        "Register a completed deliverable file from the working directory. Saves an immutable copy for this project run; call once for each file to deliver."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false})
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input["path"]
            .as_str()
            .context("artifact path is required")?;
        Ok(ToolOutput::ok(
            register(&self.archive, &ctx.working_dir, path)?.to_string(),
        ))
    }
}
pub(super) fn register(archive: &Archive, workdir: &std::path::Path, path: &str) -> Result<Value> {
    archive.check()?;
    let source = workdir.join(path).canonicalize()?;
    anyhow::ensure!(
        source.starts_with(workdir.canonicalize()?),
        "artifact must be inside the working directory"
    );
    let mut file = File::open(&source)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "artifact must be a regular file"
    );
    let id = ulid::Ulid::new().to_string();
    let target = archive.root.join(format!("artifact-{id}"));
    let saved=(||->Result<Value>{
            let mut output=OpenOptions::new().create_new(true).write(true).open(&target)?;
            let mut digest=Sha256::new();let mut size=0u64;let mut buffer=[0u8;65536];
            loop{let n=file.read(&mut buffer)?;if n==0{break;}output.write_all(&buffer[..n])?;digest.update(&buffer[..n]);size+=n as u64;}
            output.sync_all()?;
            let entry=json!({"id":id,"name":source.file_name().context("artifact filename missing")?.to_string_lossy(),"file":format!("artifact-{id}"),"size":size,"sha256":format!("{:x}",digest.finalize())});
            write_new(&archive.root.join(format!("artifact-{id}.json")),&entry)?;
            archive.event("artifact_registered",entry.clone())?;
            Ok(entry)
        })().inspect_err(|e|archive.fail(e))?;
    Ok(saved)
}
pub(super) fn install(id: &str, archive: Archive) -> opencoder_session::extensions::Registration {
    opencoder_session::extensions::register(id, vec![Arc::new(RegisterArtifact { archive })])
}
