mod brain_delivery;
mod handoff_report;
mod hub;
mod report;
#[cfg(test)]
mod scheduler_tests;
mod socket;
pub use hub::{Hub, UnregisterResult};
pub use socket::upgrade;

pub(super) enum SocketCommand {
    Frame(Box<opencoder_core::fleet::ServerFrame>),
    Close,
}
