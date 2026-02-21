#[cfg(target_os = "macos")]
use crate::encoder::EncodedPacket;

/// Common decoder interface for platform implementations.
pub trait Decoder {
    type Packet;

    fn decode(&self, packet: &Self::Packet) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
pub use crate::platform::macos::decoder::VideoToolboxDecoder;

#[cfg(target_os = "macos")]
impl Decoder for VideoToolboxDecoder {
    type Packet = EncodedPacket;

    fn decode(&self, packet: &Self::Packet) -> Result<(), String> {
        crate::platform::macos::decoder::VideoToolboxDecoder::decode(self, packet)
    }
}
