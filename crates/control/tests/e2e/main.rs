//! End-to-end coverage of the `opencoder-server` control plane: the real
//! `opencoder_control::build_app` router (bearer auth + web assets + the full
//! `/api` surface) driven over real HTTP by a scripted in-process WS node
//! (`opencoder_node::fleet::run`), matching the deploy topology of
//! `opencoder-server` + `opencoder-agent`. One module per API family; see
//! `support.rs` for the harness.

mod support;

mod admin_drain;
mod agents_api;
mod agents_resources_extra;
mod brain_api;
mod brain_dispatch_extra;
mod compat_nodes;
mod dag_dispatch_extra;
mod dag_runs;
mod executions_artifacts;
mod executions_core;
mod executions_paging;
mod executions_streams;
mod executions_submit;
mod fleet_admin_extra;
mod fleet_maintenance;
mod infra_static;
mod project_api;
mod project_crud_extra;
mod sessions_compat_extra;
mod sessions_relay;
mod support_knobs;
mod teams_dag_defs;
mod todo_templates_extra;
mod todo_workflows;
