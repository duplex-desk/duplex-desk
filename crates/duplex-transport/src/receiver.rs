use std::net::SocketAddr;

use duplex_proto::{ControlMessage, InputEvent, VideoPacket};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::error::TransportError;
use crate::packet::{Packet, PacketType};
use crate::tls::client_tls_config;

/// 解码后交给上层的视频帧
#[derive(Debug)]
pub struct VideoFrame {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub timestamp_us: u64,
}

#[derive(Debug)]
pub enum ServerPacket {
    Video(VideoFrame),
    Control(ControlMessage),
}

pub struct Receiver {
    endpoint: Endpoint,
}

impl Receiver {
    pub fn new() -> Result<Self, TransportError> {
        let tls = client_tls_config();
        let client_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(format!("quic tls error: {e}")))?;
        let client_config = quinn::ClientConfig::new(std::sync::Arc::new(client_crypto));

        // 绑定随机端口
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse().expect("parse endpoint addr"))
            .map_err(TransportError::Io)?;
        endpoint.set_default_client_config(client_config);

        Ok(Self { endpoint })
    }

    /// 连接到 Host
    pub async fn connect(&self, host_addr: SocketAddr) -> Result<ReceiverSession, TransportError> {
        let conn = self
            .endpoint
            .connect(host_addr, "duplex-transport")
            .map_err(|e| TransportError::Tls(format!("connect setup error: {e}")))?
            .await
            .map_err(TransportError::Connection)?;

        tracing::info!("connected to host: {}", host_addr);

        // Host -> Viewer：视频+控制流（同一个 stream，通过 PacketType 区分）
        // Viewer -> Host：输入+控制流（同一个 stream，通过 PacketType 区分）
        Ok(ReceiverSession {
            _conn: conn,
            video_stream: None,
            input_stream: None,
        })
    }
}

pub struct ReceiverSession {
    _conn: Connection,
    video_stream: Option<RecvStream>,
    input_stream: Option<SendStream>,
}

pub struct ReceiverVideoSession {
    _conn: Connection,
    video_stream: Option<RecvStream>,
}

pub struct ReceiverInputSession {
    _conn: Connection,
    input_stream: Option<SendStream>,
}

impl ReceiverSession {
    pub fn split(self) -> (ReceiverVideoSession, ReceiverInputSession) {
        let video_conn = self._conn.clone();
        (
            ReceiverVideoSession {
                _conn: video_conn,
                video_stream: self.video_stream,
            },
            ReceiverInputSession {
                _conn: self._conn,
                input_stream: self.input_stream,
            },
        )
    }

    /// 仅接收视频帧（兼容 API）
    pub async fn recv_video(&mut self) -> Result<VideoFrame, TransportError> {
        match self.recv_server_packet().await? {
            ServerPacket::Video(frame) => Ok(frame),
            ServerPacket::Control(_) => {
                Err(TransportError::InvalidPacketType(PacketType::Control as u8))
            }
        }
    }

    /// 接收服务端视频/控制消息
    pub async fn recv_server_packet(&mut self) -> Result<ServerPacket, TransportError> {
        self.ensure_video_stream().await?;
        recv_server_packet_from_stream(self.video_stream.as_mut().expect("stream initialized"))
            .await
    }

    /// 发送输入事件给 Host（Viewer 端调用）
    pub async fn send_input(&mut self, event: &InputEvent) -> Result<(), TransportError> {
        self.ensure_input_stream().await?;
        send_input_on_stream(
            self.input_stream.as_mut().expect("stream initialized"),
            event,
        )
        .await
    }

    /// 发送控制消息给 Host（Viewer 端调用）
    pub async fn send_control(&mut self, message: &ControlMessage) -> Result<(), TransportError> {
        self.ensure_input_stream().await?;
        send_control_on_stream(
            self.input_stream.as_mut().expect("stream initialized"),
            message,
        )
        .await
    }

    async fn ensure_video_stream(&mut self) -> Result<(), TransportError> {
        if self.video_stream.is_none() {
            self.video_stream = Some(
                self._conn
                    .accept_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }

    async fn ensure_input_stream(&mut self) -> Result<(), TransportError> {
        if self.input_stream.is_none() {
            self.input_stream = Some(
                self._conn
                    .open_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }
}

impl ReceiverVideoSession {
    /// 仅接收视频帧（兼容 API）
    pub async fn recv_video(&mut self) -> Result<VideoFrame, TransportError> {
        match self.recv_server_packet().await? {
            ServerPacket::Video(frame) => Ok(frame),
            ServerPacket::Control(_) => {
                Err(TransportError::InvalidPacketType(PacketType::Control as u8))
            }
        }
    }

    /// 接收服务端视频/控制消息
    pub async fn recv_server_packet(&mut self) -> Result<ServerPacket, TransportError> {
        self.ensure_video_stream().await?;
        recv_server_packet_from_stream(self.video_stream.as_mut().expect("stream initialized"))
            .await
    }

    async fn ensure_video_stream(&mut self) -> Result<(), TransportError> {
        if self.video_stream.is_none() {
            self.video_stream = Some(
                self._conn
                    .accept_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }
}

impl ReceiverInputSession {
    /// 发送输入事件给 Host（Viewer 端调用）
    pub async fn send_input(&mut self, event: &InputEvent) -> Result<(), TransportError> {
        self.ensure_input_stream().await?;
        send_input_on_stream(
            self.input_stream.as_mut().expect("stream initialized"),
            event,
        )
        .await
    }

    /// 发送控制消息给 Host（Viewer 端调用）
    pub async fn send_control(&mut self, message: &ControlMessage) -> Result<(), TransportError> {
        self.ensure_input_stream().await?;
        send_control_on_stream(
            self.input_stream.as_mut().expect("stream initialized"),
            message,
        )
        .await
    }

    async fn ensure_input_stream(&mut self) -> Result<(), TransportError> {
        if self.input_stream.is_none() {
            self.input_stream = Some(
                self._conn
                    .open_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }
}

async fn recv_server_packet_from_stream(
    stream: &mut RecvStream,
) -> Result<ServerPacket, TransportError> {
    // 读包头（5字节）
    let mut header = [0u8; 5];
    stream
        .read_exact(&mut header)
        .await
        .map_err(TransportError::Read)?;

    let packet_type = header[0];
    let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;

    // 读 payload
    let mut payload = vec![0u8; payload_len];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(TransportError::Read)?;

    match packet_type {
        x if x == PacketType::VideoKeyframe as u8 || x == PacketType::VideoFrame as u8 => {
            let video = VideoPacket::decode(&payload).map_err(TransportError::Serialize)?;
            let is_keyframe = x == PacketType::VideoKeyframe as u8;
            Ok(ServerPacket::Video(VideoFrame {
                data: video.data,
                is_keyframe,
                timestamp_us: video.timestamp_us,
            }))
        }
        x if x == PacketType::Control as u8 => {
            let message = ControlMessage::decode(&payload).map_err(TransportError::Serialize)?;
            Ok(ServerPacket::Control(message))
        }
        x if x == PacketType::Ping as u8 => Ok(ServerPacket::Control(ControlMessage::Ping)),
        x if x == PacketType::Pong as u8 => Ok(ServerPacket::Control(ControlMessage::Pong)),
        x => Err(TransportError::InvalidPacketType(x)),
    }
}

async fn send_input_on_stream(
    stream: &mut SendStream,
    event: &InputEvent,
) -> Result<(), TransportError> {
    let packet_type = match event {
        InputEvent::MouseMove { .. }
        | InputEvent::MouseDown { .. }
        | InputEvent::MouseUp { .. }
        | InputEvent::MouseScroll { .. } => PacketType::InputMouse,
        InputEvent::KeyDown { .. } | InputEvent::KeyUp { .. } => PacketType::InputKeyboard,
    };

    let packet = Packet::new(packet_type, event.encode());
    stream
        .write_all(&packet.encode())
        .await
        .map_err(TransportError::Write)?;

    Ok(())
}

async fn send_control_on_stream(
    stream: &mut SendStream,
    message: &ControlMessage,
) -> Result<(), TransportError> {
    let payload = message.encode().map_err(TransportError::Serialize)?;
    let packet = Packet::new(PacketType::Control, payload);
    stream
        .write_all(&packet.encode())
        .await
        .map_err(TransportError::Write)?;
    Ok(())
}
