//! Minimal `wasmtime`-compatible CLI for provisioned runc rootfs trees.
//!
//! `sandbox: runc` wasm steps launch `wasmtime run --dir=<mount> [--env K=V]
//! <module> [args…]` inside the container, so the provisioned rootfs needs a
//! wasm runtime at `/usr/bin/wasmtime`. This example assembles a small one
//! out of the SAME wasmtime/wasi crates the in-process executor embeds —
//! just enough CLI surface for the step contract — for rootfs provisioning
//! and the manual runc tests:
//!
//! ```text
//! cargo build -p opencoder-dag-runtime --example wasmtime-cli
//! cp target/debug/examples/wasmtime-cli <rootfs>/usr/bin/wasmtime
//! ```
//!
//! Differences from the real CLI are deliberate: no flags beyond
//! `run`/`--dir`/`--env`, inherited stdio, and guest exit codes pass
//! through (the runc runner owns timeouts and output limits).

use std::path::PathBuf;

use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::p1::{add_to_linker_sync, WasiP1Ctx};
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

fn main() -> anyhow::Result<()> {
    let spec = parse_args(std::env::args().skip(1))?;
    // `wasmtime::Error` deliberately does not implement std::error::Error,
    // so stringify at each boundary instead of relying on `?` conversion.
    let wrap = |e: wasmtime::Error| anyhow::anyhow!("{e}");
    let engine = Engine::default();
    // Builder defaults DISCARD stdout/stderr; a CLI must inherit them.
    let mut builder = WasiCtxBuilder::new();
    builder
        .inherit_stdout()
        .inherit_stderr()
        .args(&spec.module_argv)
        .envs(&spec.envs);
    for (host, guest) in &spec.dirs {
        builder
            .preopened_dir(host, guest, FsPerms::ReadWrite)
            .map_err(&wrap)?;
    }
    let wasi = builder.build_p1();
    let mut store = Store::new(&engine, wasi);
    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    add_to_linker_sync(&mut linker, |t| t).map_err(&wrap)?;
    let module = Module::from_file(&engine, &spec.module).map_err(&wrap)?;
    let instance = linker.instantiate(&mut store, &module).map_err(&wrap)?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(&wrap)?;
    match start.call(&mut store, ()) {
        Ok(()) => Ok(()),
        Err(err) => match err.downcast_ref::<I32Exit>() {
            Some(exit) if exit.0 != 0 => std::process::exit(exit.0),
            _ => Err(wrap(err)),
        },
    }
}

/// A parsed `run` invocation: preopened dirs, env pairs, the module path
/// and the module's own argv (argv[0] = module path, as WASI expects).
struct RunSpec {
    dirs: Vec<(PathBuf, String)>,
    envs: Vec<(String, String)>,
    module: PathBuf,
    module_argv: Vec<String>,
}

fn parse_args(argv: impl Iterator<Item = String>) -> anyhow::Result<RunSpec> {
    let mut dirs = Vec::new();
    let mut envs = Vec::new();
    let mut module_argv: Vec<String> = Vec::new();
    let mut tokens = argv.peekable();
    if tokens.peek().map(String::as_str) == Some("run") {
        tokens.next();
    }
    while let Some(token) = tokens.next() {
        if let Some(spec) = token.strip_prefix("--dir=") {
            dirs.push(match spec.split_once("::") {
                Some((host, guest)) => (PathBuf::from(host), guest.to_string()),
                None => (PathBuf::from(spec), spec.to_string()),
            });
        } else if token == "--env" || token.starts_with("--env=") {
            let pair = match token.strip_prefix("--env=") {
                Some(kp) => kp.to_string(),
                None => tokens.next().unwrap_or_default(),
            };
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--env expects K=V"))?;
            envs.push((k.to_string(), v.to_string()));
        } else if token.starts_with('-') && module_argv.is_empty() {
            // Flags only count BEFORE the module path; everything after it
            // is the module's own argv, verbatim.
            anyhow::bail!("unsupported flag: {token}");
        } else {
            module_argv.push(token);
        }
    }
    let module = module_argv
        .first()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("missing module path"))?;
    Ok(RunSpec {
        dirs,
        envs,
        module,
        module_argv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> anyhow::Result<RunSpec> {
        parse_args(items.iter().map(|s| s.to_string()))
    }

    /// The exact argv shape the OCI bundle generates must decode; unknown
    /// flags and a missing module fail closed.
    #[test]
    fn parses_the_bundle_argv_shape() {
        let spec = argv(&[
            "run",
            "--dir=/workspace/context",
            "--env",
            "OPENCODER_RUN_ID=dag-1",
            "--env=OPENCODER_STEP_DIR=/workspace/context/step",
            "/workspace/context/step/module.wasm",
            "--flag",
            "value",
        ])
        .unwrap();
        assert_eq!(
            spec.dirs,
            vec![(
                PathBuf::from("/workspace/context"),
                "/workspace/context".into()
            )]
        );
        assert_eq!(
            spec.envs,
            vec![
                ("OPENCODER_RUN_ID".into(), "dag-1".into()),
                (
                    "OPENCODER_STEP_DIR".into(),
                    "/workspace/context/step".into()
                ),
            ]
        );
        assert_eq!(
            spec.module,
            PathBuf::from("/workspace/context/step/module.wasm")
        );
        assert_eq!(spec.module_argv.len(), 3);

        // `--dir=host::guest` maps a host tree onto a different guest path.
        let spec = argv(&["--dir=/tmp/h::/workspace/context", "m.wasm"]).unwrap();
        assert_eq!(
            spec.dirs,
            vec![(PathBuf::from("/tmp/h"), "/workspace/context".into())]
        );

        assert!(argv(&["run", "--fuel=1", "m.wasm"]).is_err());
        assert!(argv(&["run"]).is_err());
    }
}
