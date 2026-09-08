//! Server-only control plane. No dependency on a session or workload runtime.
pub mod admission;
pub mod api;
mod bootstrap;
mod resource_scope;
mod routes;
pub mod transport;

// Share the existing stateless/global-definition HTTP implementations with
// the node API. Their state seam has no execution handles on the server.
#[path = "../../web/src/api_agent_nfs.rs"]
pub mod api_agent_nfs;
#[path = "../../web/src/api_agent_resources.rs"]
pub mod api_agent_resources;
#[path = "../../web/src/api_agents.rs"]
pub mod api_agents;
#[path = "../../web/src/api_brain.rs"]
pub mod api_brain;
#[path = "../../web/src/api_project.rs"]
pub mod api_project;
#[path = "../../web/src/api_project_todos.rs"]
pub mod api_project_todos;
#[path = "../../web/src/api_todo_envs.rs"]
pub mod api_todo_envs;
#[path = "../../web/src/api_todo_template_versions.rs"]
pub mod api_todo_template_versions;
#[path = "../../web/src/api_todo_templates.rs"]
pub mod api_todo_templates;
#[path = "../../web/src/api_todo_util.rs"]
pub mod api_todo_util;
#[path = "../../web/src/auth_mw.rs"]
pub mod auth_mw;
#[path = "../../web/src/html.rs"]
pub mod html;
pub use api::project_util as api_project_util;

pub use bootstrap::{new_state, new_state_with_projects, serve};
use opencoder_store::{fleet::FleetStore, ProjectStore, Store};
pub use routes::build_app;
use std::{path::PathBuf, sync::Arc};

pub struct AppState {
    pub workdir: PathBuf,
    pub store: Arc<dyn Store>,
    pub projects: Arc<dyn ProjectStore>,
    pub fleet: Arc<FleetStore>,
    pub hub: Arc<transport::Hub>,
    pub brain: opencoder_brain::Runtime,
    pub(crate) brain_gate: api::brain_dispatch::BrainGate,
    pub admission: Arc<admission::AdmissionGate>,
    /// Serializes placement plus reservation; never held waiting for a node.
    pub placement: tokio::sync::Mutex<()>,
}

impl AppState {
    /// Published resource versions affect future assignments. Running nodes
    /// retain their pinned snapshot, so there are no drains to reload here.
    pub async fn reload_agents(&self) {}
}
