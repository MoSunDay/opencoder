//! In-process wasm step execution: embedded wasmtime engine with WASI
//! preview 1, epoch-based deadline/cancel interruption, and byte-limited
//! stdout/stderr capture.
//!
//! Interruption model: cranelift-compiled code cannot be killed from the
//! outside, so the engine runs with `epoch_interruption`. A ticker thread
//! bumps the engine epoch every [`TICK_MS`]; the store's epoch deadline is
//! the step budget (explicit `timeout_secs`, else [`DEFAULT_TIMEOUT_SECS`]).
//! Cancellation is delivered by instantly bumping the epoch past the
//! deadline — the running code traps, the executor maps the trap to
//! `Cancelled` / `Error("step timeout")`.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use opencoder_dag::StepOutcome;
use tokio::io::AsyncWrite;
use tokio_util::sync::CancellationToken;
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::p1::add_to_linker_sync;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

use super::super::StepCtx;
use super::super::StepResult;
use super::{error_result, finish_from_output_json, resolve_module, step_env, CONTEXT_MOUNT};

/// Epoch tick period; the store deadline is expressed in these ticks.
const TICK_MS: u64 = 100;

/// Budget ceiling when the step sets no `timeout_secs` (also bounds the
/// cancel epoch-jump arithmetic). 24 hours.
const DEFAULT_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// Bounded tail for trap messages in `error`.
const ERROR_TAIL_BYTES: usize = 2048;

/// Execute one wasm step inside this process (blocking work off the async
/// reactor; epoch interruption honors both timeout and cancellation).
pub(crate) async fn execute(
    ctx: &StepCtx,
    run_root: &Path,
    module_library: Option<&Path>,
    tokens: &[String],
    timeout_secs: Option<u64>,
    cancel: CancellationToken,
) -> StepResult {
    let module_path = match resolve_module(run_root, module_library, &tokens[0]) {
        Ok(p) => p,
        Err(e) => return error_result(e),
    };
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let watcher = {
        let flag = Arc::clone(&cancel_flag);
        let token = cancel.clone();
        tokio::spawn(async move {
            token.cancelled().await;
            flag.store(true, Ordering::SeqCst);
        })
    };
    let run_root = run_root.to_path_buf();
    let tokens = tokens.to_vec();
    let env = step_env(ctx);
    let step_name = ctx.step.name.clone();
    let job = tokio::task::spawn_blocking(move || {
        run_sync(
            module_path,
            tokens,
            env,
            run_root,
            step_name,
            timeout_secs,
            cancel_flag,
        )
    });
    let result = match job.await {
        Ok(res) => res,
        Err(e) => error_result(format!("wasm runtime task failed: {e}")),
    };
    watcher.abort();
    result
}

/// Sync engine run — owns the whole lifecycle: engine, ticker, store,
/// instantiation, classification of the terminal error.
fn run_sync(
    module_path: PathBuf,
    tokens: Vec<String>,
    env: Vec<(String, String)>,
    run_root: PathBuf,
    step_name: String,
    timeout_secs: Option<u64>,
    cancel_flag: Arc<AtomicBool>,
) -> StepResult {
    let budget_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    let ticks = (budget_secs.saturating_mul(1000) / TICK_MS).max(1);

    let mut config = Config::new();
    config.epoch_interruption(true);
    let engine = match Engine::new(&config) {
        Ok(e) => e,
        Err(e) => return error_result(format!("wasmtime engine init failed: {e}")),
    };
    let _ticker = EpochTicker::spawn(engine.clone(), ticks, Arc::clone(&cancel_flag));

    let limit = crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES;
    let stdout = SharedSink::new(limit);
    let stderr = SharedSink::new(limit);

    let mut builder = WasiCtxBuilder::new();
    builder
        .args(&tokens)
        .envs(&env)
        .stdout(stdout.clone())
        .stderr(stderr.clone());
    if let Err(e) = builder.preopened_dir(&run_root, CONTEXT_MOUNT, FsPerms::ReadWrite) {
        return error_result(format!("cannot preopen run context root: {e}"));
    }
    let wasi = builder.build_p1();

    let mut store = Store::new(&engine, wasi);
    store.set_epoch_deadline(ticks);

    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    if let Err(e) = add_to_linker_sync(&mut linker, |t| t) {
        return error_result(format!("wasi linker init failed: {e}"));
    }
    let module = match Module::from_file(&engine, &module_path) {
        Ok(m) => m,
        Err(e) => return error_result(format!("cannot compile wasm module: {e}")),
    };
    let instance = match linker.instantiate(&mut store, &module) {
        Ok(i) => i,
        Err(e) => return error_result(format!("cannot instantiate wasm module: {e}")),
    };
    let start = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
        Ok(f) => f,
        Err(_) => {
            return error_result(
                "wasm module has no `_start` export (only WASI command modules are runnable)"
                    .into(),
            )
        }
    };

    let call = start.call(&mut store, ());
    drop(_ticker); // stop ticking before reading the captured output

    if let Some(msg) = stdout
        .overflow_message()
        .or_else(|| stderr.overflow_message())
    {
        return error_result(msg);
    }
    let output_text = format_output(&stdout.snapshot(), &stderr.snapshot());
    let step_dir = run_root.join(&step_name);
    match call {
        Ok(()) => finish_from_output_json(&step_dir, output_text),
        Err(err) => {
            if let Some(exit) = err.downcast_ref::<I32Exit>() {
                return if exit.0 == 0 {
                    finish_from_output_json(&step_dir, output_text)
                } else {
                    error_result(format!(
                        "wasm module exited with code {}:\n{}",
                        exit.0,
                        super::tail(&stderr.snapshot(), ERROR_TAIL_BYTES)
                    ))
                };
            }
            // Epoch traps surface as `wasmtime::Trap::Interrupt`; the
            // human reason ("epoch deadline reached") may hide in the
            // error chain, so check both.
            let epoch_trapped = matches!(
                err.downcast_ref::<wasmtime::Trap>(),
                Some(wasmtime::Trap::Interrupt)
            ) || format!("{err:#}").contains("epoch deadline");
            let text = err.to_string();
            if epoch_trapped {
                if cancel_flag.load(Ordering::SeqCst) {
                    return StepResult {
                        outcome: StepOutcome::Cancelled,
                        ..error_result("wasm step cancelled".into())
                    };
                }
                return error_result(format!("step timeout after {budget_secs}s"));
            }
            error_result(super::tail(&text, ERROR_TAIL_BYTES))
        }
    }
}

/// Combine captured streams into the artifact/event text.
fn format_output(stdout: &str, stderr: &str) -> String {
    if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        format!("-- stdout --\n{stdout}\n-- stderr --\n{stderr}")
    }
}

// ---------------------------------------------------------------------------
// Epoch ticker
// ---------------------------------------------------------------------------

/// Background thread that advances the engine epoch. On cancellation it
/// jumps the epoch past the store deadline so the guest traps immediately.
struct EpochTicker {
    done: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: Engine, cancel_bump_ticks: u64, cancel_flag: Arc<AtomicBool>) -> Self {
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        let handle = std::thread::spawn(move || {
            let mut jumped = false;
            while !flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(TICK_MS));
                if !jumped && cancel_flag.load(Ordering::SeqCst) {
                    // Deadline was set `ticks` beyond the epoch at store
                    // creation; a burst past that force-traps the guest.
                    for _ in 0..=cancel_bump_ticks {
                        engine.increment_epoch();
                    }
                    jumped = true;
                }
                engine.increment_epoch();
            }
        });
        EpochTicker {
            done,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Byte-limited output capture
// ---------------------------------------------------------------------------

/// Shared state behind the guest stdout/stderr sinks.
struct SinkState {
    buf: Mutex<Vec<u8>>,
    limit: usize,
    overflow: AtomicBool,
}

/// A [`StdoutStream`] capturing into a bounded buffer. Guest writes past
/// the limit fail with the `output_limit_exceeded:` marker (the same
/// contract the retired python executor used).
#[derive(Clone)]
struct SharedSink(Arc<SinkState>);

impl SharedSink {
    fn new(limit: usize) -> Self {
        SharedSink(Arc::new(SinkState {
            buf: Mutex::new(Vec::new()),
            limit,
            overflow: AtomicBool::new(false),
        }))
    }

    fn snapshot(&self) -> String {
        self.0
            .buf
            .lock()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }

    fn overflow_message(&self) -> Option<String> {
        (self.0.overflow.load(Ordering::SeqCst)).then(|| {
            format!(
                "output_limit_exceeded: wasm output exceeds {} bytes",
                self.0.limit
            )
        })
    }
}

impl wasmtime_wasi::cli::IsTerminal for SharedSink {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl wasmtime_wasi::cli::StdoutStream for SharedSink {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(SinkWriter(Arc::clone(&self.0)))
    }
}

struct SinkWriter(Arc<SinkState>);

impl AsyncWrite for SinkWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.0.overflow.load(Ordering::SeqCst) {
            return Poll::Ready(Err(limit_error(self.0.limit)));
        }
        let mut sink = match self.0.buf.lock() {
            Ok(g) => g,
            Err(_) => return Poll::Ready(Err(std::io::Error::other("wasm output sink poisoned"))),
        };
        if sink.len() + buf.len() > self.0.limit {
            self.0.overflow.store(true, Ordering::SeqCst);
            return Poll::Ready(Err(limit_error(self.0.limit)));
        }
        sink.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn limit_error(limit: usize) -> std::io::Error {
    std::io::Error::other(format!(
        "output_limit_exceeded: wasm output exceeds {limit} bytes"
    ))
}
