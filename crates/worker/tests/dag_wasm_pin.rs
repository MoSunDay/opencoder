//! End-to-end pin coverage: pool publish → accept-time pin → real wasm
//! execution from the frozen `_modules` copy → freeze across later pool
//! publishes (both the `current` form and the explicit `tool@v1.wasm`
//! version pin).

#[path = "../src/dag_wasm_pin.rs"]
mod dag_wasm_pin;

use dag_wasm_pin::pin;
use opencoder_dag::{DagSpec, StepKind, StepOutcome, StepSpec};
use opencoder_dag_runtime::{execute_wasm_step, StepCtx};

/// WAT printing one known message to stdout — mirrors the node-test
/// fixture in `tests/support/wasm.rs` (inlined so this test does not
/// depend on the support module).
fn stdout_module_wat(message: &str) -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{msg}\n")
  (func (export "_start")
    (i32.store (i32.const 64) (i32.const 0))
    (i32.store (i32.const 68) (i32.const {len}))
    (drop (call $fd_write (i32.const 1) (i32.const 64) (i32.const 1) (i32.const 72)))))"#,
        msg = message,
        len = message.len() + 1
    )
}

fn compile(message: &str) -> Vec<u8> {
    wat::parse_str(stdout_module_wat(message)).unwrap()
}

fn spec(command: &str) -> DagSpec {
    DagSpec {
        name: "e2e-workflow".into(),
        description: None,
        steps: vec![StepSpec {
            name: "run".into(),
            depends_on: vec![],
            kind: StepKind::Wasm {
                command: command.into(),
                sandbox: None,
            },
            timeout_secs: None,
        }],
    }
}

/// A `StepCtx` built exactly like the dag-runtime wasm executor fixture.
fn step_ctx(workflow_root: &std::path::Path, spec: DagSpec) -> StepCtx {
    StepCtx {
        run_id: "run-1".into(),
        step: spec.steps[0].clone(),
        spec,
        states: opencoder_dag::StepStates::new(),
        outputs: opencoder_dag::StepOutputs::new(),
        workflow_root: workflow_root.to_path_buf(),
    }
}

#[tokio::test]
async fn pinned_module_executes_and_stays_frozen_across_pool_publishes() {
    let pool = tempfile::tempdir().unwrap();
    let workflow = tempfile::tempdir().unwrap();

    // Server side: publish v1 to the pool.
    let v1 = compile("pinned-v1-message");
    opencoder_dag_wasm::save_wasm_version(pool.path(), "tool", "e2e", &v1).unwrap();

    // Node accept: partial JSON configures only the pool dir.
    let config: opencoder_core::Config = serde_json::from_value(serde_json::json!({
        "dag": {"wasm_dir": pool.path().to_string_lossy()}
    }))
    .unwrap();
    let spec = spec("tool.wasm");
    pin(&config, &spec, workflow.path()).unwrap();
    let frozen = workflow.path().join("_modules/tool.wasm");
    assert!(frozen.is_file());
    assert_eq!(std::fs::read(&frozen).unwrap(), v1);

    // Execute for real from the pinned copy.
    let result = execute_wasm_step(&step_ctx(workflow.path(), spec.clone())).await;
    assert_eq!(result.outcome, StepOutcome::Done, "{result:?}");
    assert!(
        result.output_text.contains("pinned-v1-message"),
        "{result:?}"
    );

    // FREEZE: publish a different v2 into the pool — the node's frozen
    // copy is not live and a second execution still runs v1.
    let v2 = compile("pinned-v2-message");
    opencoder_dag_wasm::save_wasm_version(pool.path(), "tool", "e2e-2", &v2).unwrap();
    assert_eq!(std::fs::read(&frozen).unwrap(), v1);
    let second = execute_wasm_step(&step_ctx(workflow.path(), spec)).await;
    assert_eq!(second.outcome, StepOutcome::Done, "{second:?}");
    assert!(
        second.output_text.contains("pinned-v1-message"),
        "{second:?}"
    );
}

#[tokio::test]
async fn explicitly_pinned_version_executes_and_ignores_current_flips() {
    let pool = tempfile::tempdir().unwrap();
    let workflow = tempfile::tempdir().unwrap();

    // Server side: v1 then v2 — `current` lands on v2.
    let v1 = compile("pinned-v1-message");
    opencoder_dag_wasm::save_wasm_version(pool.path(), "tool", "e2e", &v1).unwrap();
    let v2 = compile("pinned-v2-message");
    opencoder_dag_wasm::save_wasm_version(pool.path(), "tool", "e2e-2", &v2).unwrap();

    // Node accept with an explicit version pin: freezes v1 (NOT current).
    let config: opencoder_core::Config = serde_json::from_value(serde_json::json!({
        "dag": {"wasm_dir": pool.path().to_string_lossy()}
    }))
    .unwrap();
    let spec = spec("tool@v1.wasm");
    pin(&config, &spec, workflow.path()).unwrap();
    let frozen = workflow.path().join("_modules/tool@v1.wasm");
    assert!(frozen.is_file());
    assert_eq!(std::fs::read(&frozen).unwrap(), v1);

    // Execute for real from the pinned copy — current is v2, the run
    // observes v1.
    let result = execute_wasm_step(&step_ctx(workflow.path(), spec.clone())).await;
    assert_eq!(result.outcome, StepOutcome::Done, "{result:?}");
    assert!(
        result.output_text.contains("pinned-v1-message"),
        "{result:?}"
    );

    // `current` keeps moving (v3 published, then rollback to v2): the
    // explicit pin neither follows nor breaks — still v1 bytes/output.
    let v3 = compile("pinned-v3-message");
    opencoder_dag_wasm::save_wasm_version(pool.path(), "tool", "e2e-3", &v3).unwrap();
    opencoder_dag_wasm::rollback_wasm(pool.path(), "tool", 2).unwrap();
    assert_eq!(std::fs::read(&frozen).unwrap(), v1);
    let second = execute_wasm_step(&step_ctx(workflow.path(), spec)).await;
    assert_eq!(second.outcome, StepOutcome::Done, "{second:?}");
    assert!(
        second.output_text.contains("pinned-v1-message"),
        "{second:?}"
    );
}
