mod brain_delivery;
mod handoff_report;
mod hub;
#[cfg(test)]
mod layered_tests;
mod report;
mod socket;
mod socket_io;
mod socket_report;
pub use hub::{Hub, UnregisterResult};
pub use socket::{set_hello_timeout_for_tests, upgrade};

pub(super) enum SocketCommand {
    Frame(Box<opencoder_core::fleet::ServerFrame>),
    Close,
}
