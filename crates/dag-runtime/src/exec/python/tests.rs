//! Tests (offline; no runc, no network, no host python needed).

use super::*;
use opencoder_dag::{DagSpec, StepOutputs, StepSpec, StepStates};
use serde_json::json;

/// Fixture: `b` (the python step under test) after `a` finished Done
/// with `outputs["a"]`. `sandbox` defaults to in-process.
fn step_ctx(workflow_root: &Path, code: &str, timeout_secs: Option<u64>) -> StepCtx {
    let spec = DagSpec {
        name: "test-workflow".into(),
        description: None,
        steps: vec![
            StepSpec {
                name: "a".into(),
                depends_on: vec![],
                kind: StepKind::Python {
                    code: "pass".into(),
                    sandbox: None,
                },
                timeout_secs: None,
            },
            StepSpec {
                name: "b".into(),
                depends_on: vec!["a".into()],
                kind: StepKind::Python {
                    code: code.into(),
                    sandbox: None,
                },
                timeout_secs,
            },
        ],
    };
    let step = spec.steps[1].clone();
    let states: StepStates = [("a".to_string(), StepOutcome::Done)].into_iter().collect();
    let outputs: StepOutputs = [("a".to_string(), json!({ "count": 42, "items": [1, 2, 3] }))]
        .into_iter()
        .collect();
    // The artifacts helpers need the real run/step dirs to exist (the
    // step writes into STEP_DIR).
    let dir = step_dir(workflow_root, "run-1", "b").unwrap();
    fs::create_dir_all(&dir).unwrap();
    StepCtx {
        run_id: "run-1".into(),
        spec,
        step,
        states,
        outputs,
        workflow_root: workflow_root.to_path_buf(),
    }
}

fn temp_workflow() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("workflow");
    fs::create_dir_all(&root).unwrap();
    (tmp, root)
}

#[tokio::test]
async fn captures_stdout() {
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(&root, "print('hello')", None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(
        result.outcome,
        StepOutcome::Done,
        "unexpected outcome (error={:?})",
        result.error
    );
    assert!(
        result.output_text.contains("hello"),
        "{:?}",
        result.output_text
    );
    assert_eq!(result.output_json, None);
    drop(tmp);
}

#[tokio::test]
async fn stderr_appended_with_separator() {
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(&root, "import sys\nsys.stderr.write('warned')", None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(
        result.outcome,
        StepOutcome::Done,
        "unexpected outcome (error={:?})",
        result.error
    );
    assert!(
        result.output_text.contains("-- stderr --"),
        "{:?}",
        result.output_text
    );
    assert!(
        result.output_text.contains("warned"),
        "{:?}",
        result.output_text
    );
    drop(tmp);
}

#[tokio::test]
async fn python_stdout_accepts_exact_limit() {
    let (tmp, root) = temp_workflow();
    let code = format!(
        "print('x' * {}, end='')",
        crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES
    );
    let ctx = step_ctx(&root, &code, None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Done, "{:?}", result.error);
    assert_eq!(
        result.output_text.len(),
        crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES
    );
    drop(tmp);
}

#[tokio::test]
async fn python_stdout_rejects_one_byte_over_limit() {
    let (tmp, root) = temp_workflow();
    let code = format!(
        "print('x' * {}, end='')",
        crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES + 1
    );
    let ctx = step_ctx(&root, &code, None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Error);
    assert_eq!(
        result.error.as_deref(),
        Some("output_limit_exceeded: python stdout exceeds 8388608 bytes")
    );
    drop(tmp);
}

#[tokio::test]
async fn context_global_round_trips_upstream_output() {
    let (tmp, root) = temp_workflow();
    // Write output.json from the `context` global. The embedded VM has
    // no `json` module (see module docs), so format the (known-shape)
    // upstream value manually.
    let ctx = step_ctx(
        &root,
        "v = context[\"steps\"][\"a\"][\"json\"]\nassert context[\"steps\"][\"a\"][\"ok\"]\nassert RUN_ID == \"run-1\"\nopen(STEP_DIR + \"/output.json\", \"w\").write('{\"count\": %d, \"items\": [%d, %d, %d]}' % (v[\"count\"], v[\"items\"][0], v[\"items\"][1], v[\"items\"][2]))",
        None,
    );
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Done, "{:?}", result.error);
    assert_eq!(
        result.output_json,
        Some(json!({ "count": 42, "items": [1, 2, 3] }))
    );
    drop(tmp);
}

#[tokio::test]
async fn exception_is_error_with_traceback() {
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(&root, "1/0", None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Error);
    let error = result.error.unwrap();
    assert!(error.contains("ZeroDivisionError"), "{error}");
    drop(tmp);
}

#[tokio::test]
async fn missing_output_json_is_done_without_json() {
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(&root, "x = 1", None);
    let result = execute_python_step(&ctx).await;
    assert_eq!(
        result.outcome,
        StepOutcome::Done,
        "unexpected outcome (error={:?})",
        result.error
    );
    assert_eq!(result.output_json, None);
    drop(tmp);
}

#[tokio::test]
async fn malformed_output_json_is_error() {
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(
        &root,
        "open(STEP_DIR + \"/output.json\", \"w\").write('{nope')",
        None,
    );
    let result = execute_python_step(&ctx).await;
    assert_eq!(
        result.outcome,
        StepOutcome::Error,
        "unexpected outcome (error={:?})",
        result.error
    );
    let error = result.error.expect("error set");
    assert!(error.contains("output.json is not valid JSON"), "{error}");
    drop(tmp);
}

#[test]
fn output_json_limit_is_inclusive() {
    let (_tmp, root) = temp_workflow();
    let dir = step_dir(&root, "run-1", "b").unwrap();
    fs::create_dir_all(&dir).unwrap();
    let limit = crate::sandbox::output_limit::STRUCTURED_JSON_LIMIT_BYTES;
    let json = format!("{{\"value\":\"{}\"}}", "x".repeat(limit - 12));
    assert_eq!(json.len(), limit);
    fs::write(dir.join("output.json"), &json).unwrap();
    let exact = finish_from_output_json(&dir, String::new());
    assert_eq!(exact.outcome, StepOutcome::Done, "{:?}", exact.error);

    fs::write(dir.join("output.json"), format!("{json} ")).unwrap();
    let overflow = finish_from_output_json(&dir, String::new());
    assert_eq!(overflow.outcome, StepOutcome::Error);
    assert_eq!(
        overflow.error.as_deref(),
        Some("cannot read output.json: output_limit_exceeded: output.json exceeds 2097152 bytes")
    );
}

#[tokio::test]
async fn stdlib_import_works() {
    // `itertools` is one of the VM's core native modules (see module
    // docs for why this isn't `math`).
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(
        &root,
        "import itertools\nprint(sum(itertools.repeat(2, 3)))",
        None,
    );
    let result = execute_python_step(&ctx).await;
    assert_eq!(
        result.outcome,
        StepOutcome::Done,
        "unexpected outcome (error={:?})",
        result.error
    );
    assert!(result.output_text.contains("6"), "{:?}", result.output_text);
    drop(tmp);
}

#[tokio::test]
async fn timeout_zero_is_error() {
    // A zero budget rejects the step before starting a child process.
    let (tmp, root) = temp_workflow();
    let ctx = step_ctx(
        &root,
        "x = 0\nfor _ in range(2_000_000):\n    x = x + 1",
        Some(0),
    );
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Error);
    assert!(result.error.unwrap().contains("timeout"));
    drop(tmp);
}

#[tokio::test]
async fn runc_mode_fails_closed_without_runc_on_path() {
    let (tmp, root) = temp_workflow();
    let mut ctx = step_ctx(&root, "print('hi')", None);
    ctx.step.kind = StepKind::Python {
        code: "print('hi')".into(),
        sandbox: Some(SandboxMode::Runc),
    };
    // runc is not installed in the offline test env; if it IS (a dev
    // box), this test would run the container — guard to keep it
    // deterministic offline.
    if crate::sandbox::runc::runc_available() {
        eprintln!("skipping: runc present on this host");
        return;
    }
    let result = execute_python_step(&ctx).await;
    assert_eq!(result.outcome, StepOutcome::Error);
    assert_eq!(
        result.error.as_deref(),
        Some("runc not installed on this node")
    );
    drop(tmp);
}

#[test]
fn tail_bounds_on_char_boundary() {
    assert_eq!(tail("short", 100), "short");
    let long = "ä".repeat(2048); // 2-byte chars
    let cut = tail(&long, 100);
    assert!(cut.starts_with("...\n"));
    assert!(cut.len() <= 4 + 101);
}

#[cfg(target_os = "linux")]
async fn interrupted_vm_is_reaped(timeout: bool) {
    use tokio_util::sync::CancellationToken;
    let (_tmp, root) = temp_workflow();
    let ctx = step_ctx(&root,
        "import posix\nf = open(STEP_DIR + '/pid', 'w')\nf.write(str(posix.getpid()))\nf.close()\nwhile True:\n    pass",
        timeout.then_some(2));
    let pid_file = root.join("run-1/b/pid");
    let cancel = CancellationToken::new();
    let child_cancel = cancel.clone();
    let execution =
        tokio::spawn(async move { execute_python_step_cancellable(&ctx, child_cancel).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !pid_file.exists() || fs::read_to_string(&pid_file).unwrap_or_default().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("VM reached its infinite loop");
    let pid = fs::read_to_string(&pid_file).unwrap();
    if !timeout {
        cancel.cancel();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), execution)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        result.outcome,
        if timeout {
            StepOutcome::Error
        } else {
            StepOutcome::Cancelled
        }
    );
    assert!(result
        .error
        .unwrap()
        .contains(if timeout { "timeout" } else { "cancelled" }));
    assert!(
        !Path::new("/proc").join(pid).exists(),
        "VM must be reaped before returning a terminal result"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_kills_and_reaps_an_infinite_vm() {
    interrupted_vm_is_reaped(false).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn timeout_kills_and_reaps_an_infinite_vm() {
    interrupted_vm_is_reaped(true).await;
}
