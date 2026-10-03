//! Host-specific operations shared by local sessions and execution nodes.
pub mod fs;
pub mod shell;
mod user_dirs;
pub use user_dirs::{config_dir, data_local_dir, home_dir};
#[cfg(windows)]
mod windows_acl;

pub fn execution_kinds(dag: bool) -> Vec<crate::fleet::ExecutionKind> {
    use crate::fleet::ExecutionKind::*;
    if cfg!(windows) {
        return vec![Operator];
    }
    let mut kinds = vec![Brain, Agent, Team, Todos, Project, Maintenance, Operator];
    if dag {
        kinds.push(Dag);
    }
    kinds
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_never_advertises_container_workloads() {
        use crate::fleet::ExecutionKind;
        let kinds = super::execution_kinds(true);
        assert!(kinds.contains(&ExecutionKind::Operator));
        if cfg!(windows) {
            assert_eq!(kinds, vec![ExecutionKind::Operator]);
        } else {
            assert!(kinds.contains(&ExecutionKind::Dag));
            assert!(!super::execution_kinds(false).contains(&ExecutionKind::Dag));
        }
    }
}
