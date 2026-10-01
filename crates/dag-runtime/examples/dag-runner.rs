#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() == ["--build-info"] {
        println!(
            "{}",
            serde_json::to_string(&opencoder_core::version::build_info()).unwrap()
        );
        return;
    }
    let result =
        opencoder_dag_runtime::sandbox::supervisor::run(std::env::args().skip(1).collect()).await;
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("dag-runner: {error:#}");
            2
        }
    };
    std::process::exit(code);
}
