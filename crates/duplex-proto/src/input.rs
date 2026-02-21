use serde::{Deserialize, Serialize};

/// Normalized mouse position in [0.0, 1.0].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedPos {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEvent {
    MouseMove {
        pos: NormalizedPos,
    },
    MouseDown {
        pos: NormalizedPos,
        button: MouseButton,
    },
    MouseUp {
        pos: NormalizedPos,
        button: MouseButton,
    },
    MouseScroll {
        pos: NormalizedPos,
        delta_x: f32,
        delta_y: f32,
    },
    KeyDown {
        keycode: u32,
        modifiers: Modifiers,
    },
    KeyUp {
        keycode: u32,
        modifiers: Modifiers,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl InputEvent {
    pub fn encode(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .expect("InputEvent serialization should be infallible")
    }

    pub fn decode(data: &[u8]) -> Result<Self, String> {
        bincode::serde::decode_from_slice(data, bincode::config::standard())
            .map(|(event, _)| event)
            .map_err(|e| e.to_string())
    }
}
