use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    WaitingAuth,
    Authorized,
    Streaming,
    Rejected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    AuthRequest {
        device_name: String,
        device_code: String,
    },
    AuthDecision {
        accepted: bool,
        reason: String,
    },
    SessionState {
        state: SessionState,
    },
    Disconnect {
        reason: String,
    },
    Ping,
    Pong,
}

impl ControlMessage {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        bincode::serde::encode_to_vec(self, bincode::config::standard()).map_err(|e| e.to_string())
    }

    pub fn decode(data: &[u8]) -> Result<Self, String> {
        bincode::serde::decode_from_slice(data, bincode::config::standard())
            .map(|(msg, _)| msg)
            .map_err(|e| e.to_string())
    }
}
