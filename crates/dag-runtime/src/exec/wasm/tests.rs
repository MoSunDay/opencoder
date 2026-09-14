//! Offline tests: no runc, no network. Real wasmtime runs use small WAT
//! modules (the `wat` feature compiles them transparently).

use super::*;
use opencoder_dag::{DagSpec, StepOutputs, StepSpec, StepStates};

/// Minimal stdout-printing WASI command module.
const HELLO_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "hello wasm\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 11))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))))
"#;

/// Writes `{"answer": 42}` to `a/output.json` through the preopen at fd 3.
const OUTPUT_JSON_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "fd_close"
    (func $fd_close (param i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 32) "a/output.json")
  (data (i32.const 64) "{\"answer\": 42}")
  (func (export "_start")
    ;; path_open(preopen=3, dirflags=0, path, 13, oflags=CREAT|TRUNC (9),
    ;; rights_base=fd_write (1<<6), rights_inheriting=0, fdflags=0, *fd)
    (i32.store (i32.const 0) (i32.const 0))
    (drop (call $path_open
      (i32.const 3) (i32.const 0) (i32.const 32) (i32.const 13)
      (i32.const 9) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 0)))
    (i32.store (i32.const 16) (i32.const 64))
    (i32.store (i32.const 20) (i32.const 14))
    (drop (call $fd_write (i32.load (i32.const 0)) (i32.const 16) (i32.const 1) (i32.const 24)))
    (drop (call $fd_close (i32.load (i32.const 0))))))
"#;

/// Writes "boom\n" to fd 2 (stderr) and exits 0.
const STDERR_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 8) "boom\n")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 8))
    (i32.store (i32.const 4) (i32.const 5))
    (drop (call $fd_write (i32.const 2) (i32.const 0) (i32.const 1) (i32.const 20)))))
"#;

/// proc_exit(7).
const EXIT_7_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
  (memory (export "memory") 1)
  (func (export "_start") (call $proc_exit (i32.const 7))))
"#;

/// Infinite loop — only epoch interruption ends it.
const SPIN_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "_start") (loop $l br $l)))
"#;

/// Fixture: wasm step `a` (no upstreams) under a fresh workflow root.
fn step_ctx(workflow_root: &std::path::Path, command: &str, timeout_secs: Option<u64>) -> StepCtx {
    let spec = DagSpec {
        name: "test-workflow".into(),
        description: None,
        steps: vec![StepSpec {
            name: "a".into(),
            depends_on: vec![],
            kind: StepKind::Wasm {
                command: command.into(),
                sandbox: None,
            },
            timeout_secs,
        }],
    };
    let step = spec.steps[0].clone();
    StepCtx {
        run_id: "run-1".into(),
        spec,
        step,
        states: StepStates::new(),
        outputs: StepOutputs::new(),
        workflow_root: workflow_root.to_path_buf(),
        log: None,
    }
}

fn temp_workflow_with_module(wat: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("workflow");
    let run_root = root.join("run-1");
    std::fs::create_dir_all(run_root.join("a")).unwrap();
    std::fs::write(run_root.join("tool.wat"), wat).unwrap();
    (tmp, root)
}

#[test]
fn split_command_drops_extra_whitespace() {
    assert!(split_command("   ").is_empty());
    assert_eq!(
        split_command("  tool.wasm   --flag  x  "),
        vec!["tool.wasm", "--flag", "x"]
    );
}

#[test]
fn module_paths_are_confined_to_the_context_root() {
    let run_root = std::path::Path::new("/tmp/wf/run-1");
    assert_eq!(
        module_host_path(run_root, "tool.wasm").unwrap(),
        run_root.join("tool.wasm")
    );
    assert_eq!(
        module_host_path(run_root, "build/out.wasm").unwrap(),
        run_root.join("build/out.wasm")
    );
    assert!(module_host_path(run_root, "/etc/passwd").is_err());
    assert!(module_host_path(run_root, "../escape.wasm").is_err());
    assert!(module_host_path(run_root, "a/../../escape.wasm").is_err());
}

#[test]
fn step_env_points_at_the_context_contract() {
    let (tmp, root) = temp_workflow_with_module(HELLO_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let env = step_env(&ctx);
    let get = |k: &str| env.iter().find(|(n, _)| n == k).unwrap().1.clone();
    assert_eq!(get("OPENCODER_RUN_ID"), "run-1");
    assert_eq!(get("OPENCODER_STEP_DIR"), "/workspace/context/a/");
    assert_eq!(
        get("OPENCODER_STEP_CONTEXT"),
        "/workspace/context/a/context.json"
    );
    drop(tmp);
}

#[tokio::test]
async fn hello_wasm_step_captures_stdout_and_context_json() {
    let (tmp, root) = temp_workflow_with_module(HELLO_WAT);
    let ctx = step_ctx(&root, "tool.wat --flag", None);
    let res = execute_wasm_step(&ctx).await;
    assert_eq!(res.outcome, StepOutcome::Done, "{res:?}");
    assert!(res.output_text.contains("hello wasm"), "{res:?}");
    assert!(res.output_json.is_none());
    // The context file contract is materialized in the step dir.
    let ctx_file = root.join("run-1/a/context.json");
    assert!(ctx_file.is_file());
    let body: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ctx_file).unwrap()).unwrap();
    assert_eq!(body["steps"], serde_json::json!({}));
    drop(tmp);
}

#[tokio::test]
async fn wasm_step_output_json_becomes_structured_output() {
    let (tmp, root) = temp_workflow_with_module(OUTPUT_JSON_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let res = execute_wasm_step(&ctx).await;
    assert_eq!(res.outcome, StepOutcome::Done, "{res:?}");
    assert_eq!(res.output_json.unwrap()["answer"], serde_json::json!(42));
    drop(tmp);
}

#[tokio::test]
async fn nonzero_exit_code_is_an_error_outcome() {
    let (tmp, root) = temp_workflow_with_module(EXIT_7_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let res = execute_wasm_step(&ctx).await;
    assert_eq!(res.outcome, StepOutcome::Error, "{res:?}");
    assert!(
        res.error.as_deref().unwrap().contains("exited with code 7"),
        "{res:?}"
    );
    drop(tmp);
}

#[tokio::test]
async fn timeout_secs_traps_via_epoch_deadline() {
    let (tmp, root) = temp_workflow_with_module(SPIN_WAT);
    let ctx = step_ctx(&root, "tool.wat", Some(1));
    let res = execute_wasm_step(&ctx).await;
    assert_eq!(res.outcome, StepOutcome::Error, "{res:?}");
    assert!(
        res.error
            .as_deref()
            .unwrap()
            .contains("step timeout after 1s"),
        "{res:?}"
    );
    drop(tmp);
}

#[tokio::test]
async fn cancel_token_maps_to_cancelled_outcome() {
    let (tmp, root) = temp_workflow_with_module(SPIN_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let token = tokio_util::sync::CancellationToken::new();
    let tok = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tok.cancel();
    });
    let res = execute_wasm_step_cancellable(&ctx, token).await;
    assert_eq!(res.outcome, StepOutcome::Cancelled, "{res:?}");
    drop(tmp);
}

/// In-memory node store for the `step_output` mirroring assertions.
async fn memory_store() -> std::sync::Arc<dyn opencoder_store::Store> {
    std::sync::Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap())
}

/// Run one wasm step with its output mirrored, then close the log so the
/// tail batch is guaranteed to be appended before the rows are read.
async fn run_mirrored(
    ctx: &StepCtx,
    store: &std::sync::Arc<dyn opencoder_store::Store>,
) -> StepResult {
    // `session_events.session_id` is a foreign key: the run's session has to
    // exist before the mirror can append to it (the runtime guarantees this
    // for whole runs; this harness drives the executor directly).
    store
        .create_session(&opencoder_store::SessionMeta {
            id: ctx.run_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    let log = crate::step_log::StepOutputLog::new(store.clone(), &ctx.run_id, &ctx.step.name);
    let result = execute_wasm_step_logged(ctx, CancellationToken::new(), Some(log.clone())).await;
    log.close().await;
    result
}

#[tokio::test]
async fn guest_stdout_is_mirrored_as_step_output_events() {
    let store = memory_store().await;
    let (tmp, root) = temp_workflow_with_module(HELLO_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let res = run_mirrored(&ctx, &store).await;
    assert_eq!(res.outcome, StepOutcome::Done, "{res:?}");

    // The rows live on the RUN's session and name the step (LOCKED contract).
    let rows = store.as_ref().events_after(&ctx.run_id, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.session_id, "run-1");
    assert_eq!(row.kind, opencoder_store::EventKind::Step);
    assert_eq!(row.sse_kind.as_deref(), Some("step_output"));
    assert!(row.seq.is_some(), "the store assigns a sequence");
    assert_eq!(row.payload["step"], serde_json::json!("a"));
    assert_eq!(row.payload["stream"], serde_json::json!("stdout"));
    assert_eq!(row.payload["text"], serde_json::json!("hello wasm\n"));
    assert_eq!(row.payload["at_ms"].as_i64().unwrap(), row.ts);
    drop(tmp);
}

#[tokio::test]
async fn guest_stderr_is_mirrored_with_its_stream_label() {
    let store = memory_store().await;
    let (tmp, root) = temp_workflow_with_module(STDERR_WAT);
    let ctx = step_ctx(&root, "tool.wat", None);
    let res = run_mirrored(&ctx, &store).await;
    assert_eq!(res.outcome, StepOutcome::Done, "{res:?}");
    let rows = store.as_ref().events_after(&ctx.run_id, 0).await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].payload["stream"], serde_json::json!("stderr"));
    assert_eq!(rows[0].payload["text"], serde_json::json!("boom\n"));
    // The step's own artifacts still carry the combined, bounded output.
    assert!(res.output_text.contains("boom"), "{res:?}");
    drop(tmp);
}

#[tokio::test]
async fn missing_module_is_a_clean_error() {
    let (tmp, root) = temp_workflow_with_module(HELLO_WAT);
    let ctx = step_ctx(&root, "nope.wat", None);
    let res = execute_wasm_step(&ctx).await;
    assert_eq!(res.outcome, StepOutcome::Error, "{res:?}");
    assert!(
        res.error.as_deref().unwrap().contains("module not found"),
        "{res:?}"
    );
    drop(tmp);
}
