mod brain_delivery;
mod handoff_report;
mod hub;
#[cfg(test)]
mod layered_tests;
mod report;
mod socket;
mod socket_report;
pub use hub::{Hub, UnregisterResult};
pub use socket::upgrade;

pub(super) enum SocketCommand {
    Frame(Box<opencoder_core::fleet::ServerFrame>),
    Close,
}
