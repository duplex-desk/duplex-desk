use std::sync::mpsc::Receiver;

use duplex_scap::frame::DuplexScapFrame;

use crate::EncodedPacket;

pub struct PlatformVideoEncoder;

impl PlatformVideoEncoder {
    pub fn new(
        _width: u32,
        _height: u32,
        _fps: u64,
        _bitrate_kbps: u32,
    ) -> Result<(Self, Receiver<EncodedPacket>), String> {
        Err("video encode is not supported on this platform".to_string())
    }

    pub fn encode(&self, _frame: &DuplexScapFrame) -> Result<(), String> {
        Err("video encode is not supported on this platform".to_string())
    }
}

pub struct PlatformVideoDecoder;

impl PlatformVideoDecoder {
    pub fn from_keyframe(
        _packet: &EncodedPacket,
    ) -> Result<(Self, Receiver<DuplexScapFrame>), String> {
        Err("video decode is not supported on this platform".to_string())
    }

    pub fn decode(&self, _packet: &EncodedPacket) -> Result<(), String> {
        Err("video decode is not supported on this platform".to_string())
    }
}
