//! Binary TODO file transfer. File bytes never pass through JSON or stdout logs.
use crate::{
    ctx::Ctx,
    http::{self, RequestPlan},
    out,
};
use anyhow::{Context, Result};
use clap::Subcommand;
use futures::StreamExt;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

#[derive(Subcommand, Debug)]
pub enum FilesCmd {
    List {
        id: String,
    },
    Upload {
        id: String,
        file: PathBuf,
        /// Relative destination; defaults to the local filename.
        #[arg(long)]
        path: Option<String>,
    },
    Download {
        id: String,
        path: String,
        #[arg(long, short)]
        output: PathBuf,
    },
    Delete {
        id: String,
        path: String,
    },
}

fn endpoint(id: &str) -> String {
    format!("/api/project/todos/{}", http::urlencode(id))
}

pub(super) fn plan(command: &FilesCmd) -> Result<RequestPlan> {
    Ok(match command {
        FilesCmd::List { id } => RequestPlan::get(format!("{}/files", endpoint(id))),
        FilesCmd::Delete { id, path } => {
            RequestPlan::delete(format!("{}/file", endpoint(id))).with("path", path)
        }
        FilesCmd::Download { id, path, .. } => {
            RequestPlan::get(format!("{}/file", endpoint(id))).with("path", path)
        }
        FilesCmd::Upload { id, file, path } => {
            let path = path
                .as_deref()
                .or_else(|| file.file_name().and_then(|v| v.to_str()))
                .context("destination path required")?;
            RequestPlan::put(format!("{}/file", endpoint(id))).with("path", path)
        }
    })
}

pub(super) async fn run(ctx: &Ctx, command: &FilesCmd) -> Result<i32> {
    let request = plan(command)?;
    match command {
        FilesCmd::Upload { file, .. } => {
            let file = tokio::fs::File::open(file)
                .await
                .context("open upload file")?;
            let metadata = file.metadata().await?;
            anyhow::ensure!(metadata.is_file(), "upload requires a regular file");
            let mut builder = ctx
                .http
                .put(request.url(&ctx.server))
                .header("content-type", "application/octet-stream")
                .header("content-length", metadata.len());
            if let Some(token) = &ctx.token {
                builder = builder.bearer_auth(token);
            }
            let response = builder
                .body(reqwest::Body::wrap_stream(
                    tokio_util::io::ReaderStream::new(file),
                ))
                .send()
                .await?;
            let status = response.status().as_u16();
            let body = response.text().await?;
            if !(200..300).contains(&status) {
                eprintln!("HTTP {status}: {body}");
                return Ok(http::exit_code(status));
            }
            out::json(&serde_json::from_str::<serde_json::Value>(&body)?);
            Ok(0)
        }
        FilesCmd::Download { output, .. } => {
            let response = http::send_streaming(ctx, &request).await?;
            let status = response.status().as_u16();
            if !response.status().is_success() {
                eprintln!("HTTP {status}: {}", response.text().await?);
                return Ok(http::exit_code(status));
            }
            // A failed download never overwrites an existing local file or publishes partial bytes.
            anyhow::ensure!(!output.try_exists()?, "output already exists");
            let temporary = output.with_extension(format!("part-{}", ulid::Ulid::new()));
            let result = async {
                let expected = response.content_length();
                let mut file = tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)
                    .await?;
                let mut stream = response.bytes_stream();
                let mut bytes = 0u64;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk?;
                    bytes += chunk.len() as u64;
                    file.write_all(&chunk).await?;
                }
                anyhow::ensure!(
                    expected.is_none_or(|size| size == bytes),
                    "incomplete file download"
                );
                file.sync_all().await?;
                drop(file);
                tokio::fs::hard_link(&temporary, output).await?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            let _ = tokio::fs::remove_file(temporary).await;
            result?;
            Ok(0)
        }
        _ => crate::cmd::exec_plan(ctx, request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(subcommand)]
        command: FilesCmd,
    }
    #[test]
    fn parses_and_routes_binary_file_commands() {
        let command = Cli::try_parse_from([
            "files",
            "upload",
            "t",
            "local.bin",
            "--path",
            "日志/a b.bin",
        ])
        .unwrap()
        .command;
        let plan = plan(&command).unwrap();
        assert_eq!(plan.method, reqwest::Method::PUT);
        assert!(plan.url("http://localhost").contains("path=%E6%97%A5"));
        for args in [
            vec!["files", "list", "t"],
            vec!["files", "delete", "t", "a"],
            vec!["files", "download", "t", "a", "--output", "local"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from(["files", "download", "t", "a"]).is_err());
    }
}
