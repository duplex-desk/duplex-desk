use std::net::SocketAddr;

use duplex_proto::VideoTrace;
use duplex_scap::frame::DuplexScapFrame;

#[derive(Debug)]
pub struct FrameTelemetry {
    pub frame_id: u64,
    pub host_trace: Option<VideoTrace>,
    pub viewer_recv_us: u64,
    pub viewer_decode_submit_us: u64,
    pub viewer_decode_done_us: u64,
}

#[derive(Debug)]
pub struct UiFrame {
    pub frame: DuplexScapFrame,
    pub telemetry: Option<FrameTelemetry>,
}

#[derive(Debug)]
pub enum TaskEvent {
    Frame(UiFrame),
    HostStarted(SocketAddr),
    HostAwaitingApproval {
        remote_addr: SocketAddr,
        device_name: String,
    },
    HostSessionEnded {
        message: String,
        peer_cancelled: bool,
    },
    HostStopped(String),
    ViewerConnected(SocketAddr),
    ViewerAuthResult {
        accepted: bool,
        reason: String,
    },
    ViewerStopped(String),
}
