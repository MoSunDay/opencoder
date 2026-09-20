//! Measure a private snapshot without creating an execution or changing source.
#[path = "../src/resources/snapshot.rs"]
mod snapshot;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "usage: resource_snapshot SOURCE NEW_DESTINATION"
    );
    let source = std::path::Path::new(&args[0]);
    let destination = std::path::Path::new(&args[1]);
    anyhow::ensure!(!destination.exists(), "benchmark destination must be new");
    let started = std::time::Instant::now();
    snapshot::pin(Some(source), destination)?;
    println!(
        "{}",
        serde_json::json!({"elapsed_ms":started.elapsed().as_millis(),"destination":destination})
    );
    Ok(())
}
