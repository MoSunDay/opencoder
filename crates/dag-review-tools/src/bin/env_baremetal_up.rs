//! env_baremetal_up — review-DAG env step: bring the baremetal_deploy environment up via
//! the node's `env_baremetal_deploy` op and gate on its ready endpoint.
//!
//! wasm-only: on any other target this binary refuses to run (exit 2).
//! Build with `cargo build -p opencoder-dag-review-tools --target
//! wasm32-wasip1` and publish the module into the `env_baremetal_up` wasm pool.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("env_baremetal_up is a DAG wasm step module; build for wasm32-wasip1");
    std::process::exit(2);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    let step_dir = opencoder_dag_review_tools::step_dir_from_env();
    opencoder_dag_review_tools::host::deploy_and_probe(
        &step_dir,
        "env_baremetal_deploy",
        "/healthz",
    );
}
