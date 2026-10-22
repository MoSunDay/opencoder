use super::*;

pub(super) async fn prepare_create(
    worker: &Worker,
    assignment: &Assignment,
    lease: preparation::Lease,
) -> Result<(Result<Config>, preparation::Lease)> {
    // Capture caller-scoped configuration before slow resource I/O crosses threads.
    let mut timing =
        crate::operations::admission::timing::Timing::new(&assignment.index.id, "preflight");
    let config = worker.configuration_for(assignment.request.kind)?;
    timing.mark("configuration");
    let source = opencoder_core::agent::agents_dir();
    let worker = worker.clone();
    let assignment = assignment.clone();
    preparation::run(lease, move || {
        timing.mark("blocking_worker_wait");
        prepare_with_config(&worker, &assignment, false, config, source)
    })
    .await
}
