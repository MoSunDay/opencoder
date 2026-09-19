//! harness_runner — review-DAG harness step: run the node's
//! `harness_run` op, aggregate its NDJSON stage report, and gate the
//! release on every stage passing.
//!
//! `--suite quick|full` only picks which op id the node registered
//! (`harness_run_quick` / `harness_run_full`); default `full`.
//!
//! wasm-only: on any other target this binary refuses to run (exit 2).

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("harness_runner is a DAG wasm step module; build for wasm32-wasip1");
    std::process::exit(2);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    let step_dir = opencoder_dag_review_tools::step_dir_from_env();
    let suite = std::env::args().nth(1).unwrap_or_default();
    let op_id = match suite.as_str() {
        "quick" => "harness_run_quick",
        _ => "harness_run_full",
    };
    opencoder_dag_review_tools::host::run_harness(&step_dir, op_id);
}
