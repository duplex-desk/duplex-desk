/// Stream ID 约定
pub const STREAM_VIDEO: u64 = 0;
pub const STREAM_INPUT: u64 = 1;
pub const STREAM_CONTROL: u64 = 2;

/// 包类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    VideoKeyframe = 0x01, // 关键帧（含 SPS+PPS）
    VideoFrame = 0x02,    // P 帧
    InputMouse = 0x10,    // 鼠标事件
    InputKeyboard = 0x11, // 键盘事件
    Control = 0x12,       // 控制消息（鉴权/会话状态）
    Ping = 0x20,
    Pong = 0x21,
}

/// 统一包格式
/// 线上格式：[type(1B)][len(4B 大端)][payload]
#[derive(Debug, Clone)]
pub struct Packet {
    pub packet_type: PacketType,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn new(packet_type: PacketType, payload: Vec<u8>) -> Self {
        Self {
            packet_type,
            payload,
        }
    }

    /// 序列化成字节流
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        buf.push(self.packet_type as u8);
        let len = self.payload.len() as u32;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// 从字节流解析，返回 (packet, consumed_bytes)
    /// 数据不够时返回 None（等待更多数据）
    pub fn decode(buf: &[u8]) -> Option<(Self, usize)> {
        if buf.len() < 5 {
            return None;
        }

        let packet_type = match buf[0] {
            0x01 => PacketType::VideoKeyframe,
            0x02 => PacketType::VideoFrame,
            0x10 => PacketType::InputMouse,
            0x11 => PacketType::InputKeyboard,
            0x12 => PacketType::Control,
            0x20 => PacketType::Ping,
            0x21 => PacketType::Pong,
            _ => return None,
        };

        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if buf.len() < 5 + len {
            return None;
        }

        let payload = buf[5..5 + len].to_vec();
        Some((
            Self {
                packet_type,
                payload,
            },
            5 + len,
        ))
    }
}
