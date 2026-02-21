use std::sync::mpsc::{self, Receiver};

use duplex_scap::{
    decoder::VideoToolboxDecoder,
    encoder::{EncodedPacket as VtPacket, VideoToolboxEncoder},
    frame::DuplexScapFrame,
};

use crate::EncodedPacket;

pub struct PlatformVideoEncoder {
    inner: VideoToolboxEncoder,
}

impl PlatformVideoEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps: u64,
        bitrate_kbps: u32,
    ) -> Result<(Self, Receiver<EncodedPacket>), String> {
        let (inner, vt_rx) = VideoToolboxEncoder::new(width, height, fps, bitrate_kbps)?;
        let (tx, rx) = mpsc::sync_channel::<EncodedPacket>(10);

        std::thread::spawn(move || {
            while let Ok(packet) = vt_rx.recv() {
                let mapped = EncodedPacket {
                    data: packet.data,
                    is_keyframe: packet.is_keyframe,
                    timestamp_us: packet.timestamp_us,
                };
                if tx.send(mapped).is_err() {
                    break;
                }
            }
        });

        Ok((Self { inner }, rx))
    }

    pub fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String> {
        self.inner.encode(frame)
    }
}

pub struct PlatformVideoDecoder {
    inner: VideoToolboxDecoder,
}

impl PlatformVideoDecoder {
    pub fn from_keyframe(
        packet: &EncodedPacket,
    ) -> Result<(Self, Receiver<DuplexScapFrame>), String> {
        let vt_packet = VtPacket {
            data: packet.data.clone(),
            is_keyframe: packet.is_keyframe,
            timestamp_us: packet.timestamp_us,
        };
        let (inner, rx) = VideoToolboxDecoder::from_keyframe(&vt_packet)?;
        Ok((Self { inner }, rx))
    }

    pub fn decode(&self, packet: &EncodedPacket) -> Result<(), String> {
        let vt_packet = VtPacket {
            data: packet.data.clone(),
            is_keyframe: packet.is_keyframe,
            timestamp_us: packet.timestamp_us,
        };
        self.inner.decode(&vt_packet)
    }
}
