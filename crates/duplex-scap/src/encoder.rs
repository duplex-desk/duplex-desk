use crate::frame::DuplexScapFrame;
use duplex_proto::VideoPacket;

/// Common encoder interface for platform implementations.
pub trait Encoder {
    type Packet;

    fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
pub use crate::platform::macos::encoder::{EncodedPacket, VideoToolboxEncoder};

#[cfg(target_os = "macos")]
impl Encoder for VideoToolboxEncoder {
    type Packet = EncodedPacket;

    fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String> {
        crate::platform::macos::encoder::VideoToolboxEncoder::encode(self, frame)
    }
}

#[cfg(target_os = "macos")]
impl EncodedPacket {
    pub fn to_video_packet(&self) -> VideoPacket {
        VideoPacket {
            timestamp_us: self.timestamp_us,
            data: self.data.clone(),
        }
    }

    pub fn from_video_packet(packet: VideoPacket, is_keyframe: bool) -> Self {
        Self {
            data: packet.data,
            is_keyframe,
            timestamp_us: packet.timestamp_us,
        }
    }
}
