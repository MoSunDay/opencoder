//! Server task bridge. Server owns execution, harness selection and history;
//! TUI keeps only a bookmark and renders the shared session event protocol.
pub mod client;
mod questions;
mod selection;
mod session;
pub mod stream;
pub mod transcript;
mod worker;
pub use selection::{catalog, select};
pub(crate) use selection::{poll_ui, request, Selection};
pub(crate) use session::{create, load};
pub(crate) use worker::spawn;

pub async fn admit(
    store: &dyn opencoder_store::Store,
    input: &opencoder_store::SessionInput,
    proxy: Option<&str>,
) -> anyhow::Result<i64> {
    let runtime = store.harness_runtime(&input.session_id).await?;
    let Some(remote) = runtime.and_then(|runtime| runtime.remote) else {
        return store.admit_input(input).await;
    };
    anyhow::ensure!(
        remote.created,
        "Remote execution has not accepted its first prompt yet"
    );
    let client = client::ServerClient::new(&remote.server_url, proxy)?;
    let action = match input.delivery {
        opencoder_store::Delivery::Steer => "steer",
        opencoder_store::Delivery::Queue => "queue",
    };
    let result = client
        .command(
            &input.session_id,
            action,
            serde_json::json!({
                "prompt":input.prompt,"images":input.images,"input_id":input.id,
            }),
        )
        .await?;
    result["admitted_seq"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("Server did not return an input receipt"))
}

/// Controls whose implementation reads/mutates a local runner.
pub(crate) fn local_control(action: &crate::key_handler::KeyAction) -> bool {
    use crate::key_handler::KeyAction;
    if let KeyAction::Submit(text) | KeyAction::Queue(text) | KeyAction::Steer(text) = action {
        if opencoder_session::control_cmd::is_mode_control(text) {
            return true;
        }
    }
    matches!(
        action,
        KeyAction::SidecarAsk(_)
            | KeyAction::SubagentSteer(_)
            | KeyAction::SwitchAgent(_)
            | KeyAction::ArmClearConfirm { .. }
            | KeyAction::EnterPlanEdit
            | KeyAction::SetSkill(_)
            | KeyAction::Bash(_)
    )
}

#[cfg(test)]
mod tests;
