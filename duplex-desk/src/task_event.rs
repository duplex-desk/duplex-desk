use std::net::SocketAddr;

use duplex_scap::frame::DuplexScapFrame;

#[derive(Debug)]
pub enum TaskEvent {
    Frame(DuplexScapFrame),
    HostStarted(SocketAddr),
    HostAwaitingApproval {
        remote_addr: SocketAddr,
        device_name: String,
    },
    HostStopped(String),
    ViewerConnected(SocketAddr),
    ViewerAuthResult {
        accepted: bool,
        reason: String,
    },
    ViewerStopped(String),
}
