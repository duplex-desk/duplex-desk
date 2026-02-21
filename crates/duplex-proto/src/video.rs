use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoTrace {
    pub host_capture_us: u64,
    pub host_encode_submit_us: u64,
    pub host_encode_done_us: u64,
    pub host_send_submit_us: u64,
}

/// Encoded video packet payload (Annex-B H.264 bytes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoPacket {
    pub timestamp_us: u64,
    #[serde(default)]
    pub frame_id: u64,
    #[serde(default)]
    pub trace: Option<VideoTrace>,
    pub data: Vec<u8>,
}

impl VideoPacket {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        bincode::serde::encode_to_vec(self, bincode::config::standard()).map_err(|e| e.to_string())
    }

    pub fn decode(data: &[u8]) -> Result<Self, String> {
        bincode::serde::decode_from_slice(data, bincode::config::standard())
            .map(|(packet, _)| packet)
            .map_err(|e| e.to_string())
    }
}
