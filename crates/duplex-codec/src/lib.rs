use std::sync::mpsc::Receiver;

use duplex_proto::VideoPacket;
use duplex_scap::frame::DuplexScapFrame;

mod platform;

#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub timestamp_us: u64,
}

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

pub struct VideoEncoder(platform::PlatformVideoEncoder);

impl VideoEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps: u64,
        bitrate_kbps: u32,
    ) -> Result<(Self, Receiver<EncodedPacket>), String> {
        let (inner, rx) = platform::PlatformVideoEncoder::new(width, height, fps, bitrate_kbps)?;
        Ok((Self(inner), rx))
    }

    pub fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String> {
        self.0.encode(frame)
    }
}

pub struct VideoDecoder(platform::PlatformVideoDecoder);

impl VideoDecoder {
    pub fn from_keyframe(
        packet: &EncodedPacket,
    ) -> Result<(Self, Receiver<DuplexScapFrame>), String> {
        let (inner, rx) = platform::PlatformVideoDecoder::from_keyframe(packet)?;
        Ok((Self(inner), rx))
    }

    pub fn decode(&self, packet: &EncodedPacket) -> Result<(), String> {
        self.0.decode(packet)
    }
}
