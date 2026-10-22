//! Bound allocator retention across idle execution-runtime worker threads.
use crate::AgentCommand;

const MAX_DEFAULT_WORKERS: usize = 8;

pub(crate) fn build(command: Option<&AgentCommand>) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    let configured = std::env::var_os("TOKIO_WORKER_THREADS").is_some();
    if let Some(workers) = default_workers(command, available, configured) {
        builder.worker_threads(workers);
    }
    builder.enable_all().build()
}

fn default_workers(
    command: Option<&AgentCommand>,
    available: usize,
    configured: bool,
) -> Option<usize> {
    // Explicit Tokio configuration keeps its native validation and precedence.
    // This pool polls futures; max_runs, DAG concurrency and subprocess capacity
    // are independent. Host and offline commands retain their existing defaults.
    if configured
        || !matches!(
            command,
            None | Some(AgentCommand::Run | AgentCommand::Runtime { .. })
        )
    {
        return None;
    }
    Some(available.clamp(1, MAX_DEFAULT_WORKERS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_defaults_follow_small_hosts_and_bound_large_hosts() {
        for command in [
            None,
            Some(AgentCommand::Run),
            Some(AgentCommand::Runtime { port: 1 }),
        ] {
            for (available, expected) in [(1, 1), (4, 4), (8, 8), (64, 8), (256, 8)] {
                assert_eq!(
                    default_workers(command.as_ref(), available, false),
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn explicit_thread_configuration_and_host_defaults_are_preserved() {
        for command in [
            None,
            Some(AgentCommand::Run),
            Some(AgentCommand::Runtime { port: 1 }),
        ] {
            assert_eq!(default_workers(command.as_ref(), 64, true), None);
        }
        let host = AgentCommand::Host {
            port: 1,
            standby: false,
        };
        assert_eq!(default_workers(Some(&host), 64, false), None);
        let supervisor = AgentCommand::InternalProcessSupervisor {
            runc_root: None,
            runc_id: None,
            command: Vec::new(),
        };
        assert_eq!(default_workers(Some(&supervisor), 64, false), None);
    }
}
