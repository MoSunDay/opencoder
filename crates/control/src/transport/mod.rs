mod hub;
mod report;
mod socket;
pub use hub::Hub;
pub use socket::upgrade;

pub(super) enum SocketCommand {
    Frame(Box<opencoder_core::fleet::ServerFrame>),
    Close,
}
