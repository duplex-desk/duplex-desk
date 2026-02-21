use std::net::SocketAddr;

use duplex_proto::{ControlMessage, InputEvent, VideoPacket, VideoTrace};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::error::TransportError;
use crate::packet::{Packet, PacketType};
use crate::tls::{SelfSignedCert, server_tls_config};

#[derive(Debug)]
pub enum ClientPacket {
    Input(InputEvent),
    Control(ControlMessage),
}

pub struct Sender {
    endpoint: Endpoint,
}

impl Sender {
    /// 创建发送端，监听指定端口
    pub async fn bind(addr: SocketAddr) -> Result<Self, TransportError> {
        let cert = SelfSignedCert::generate("duplex-transport").map_err(TransportError::Tls)?;
        let tls = server_tls_config(&cert).map_err(TransportError::Tls)?;

        let quic_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(format!("quic tls error: {e}")))?;
        let server_config = quinn::ServerConfig::with_crypto(std::sync::Arc::new(quic_crypto));
        let endpoint = Endpoint::server(server_config, addr).map_err(TransportError::Io)?;

        tracing::info!("sender listening on {}", addr);
        Ok(Self { endpoint })
    }

    /// 等待一个 Viewer 连接进来
    pub async fn accept(&self) -> Result<SenderSession, TransportError> {
        let conn = self
            .endpoint
            .accept()
            .await
            .ok_or(TransportError::EndpointClosed)?
            .await
            .map_err(TransportError::Connection)?;

        tracing::info!("viewer connected: {}", conn.remote_address());

        // Host -> Viewer：视频+控制流（同一个 stream，通过 PacketType 区分）
        let video_stream = conn.open_uni().await.map_err(TransportError::Connection)?;

        // Viewer -> Host：输入+控制流（同一个 stream，通过 PacketType 区分）
        Ok(SenderSession {
            _conn: conn,
            video_stream,
            input_stream: None,
        })
    }
}

pub struct SenderSession {
    _conn: Connection,
    video_stream: SendStream,
    input_stream: Option<RecvStream>,
}

pub struct SenderVideoSession {
    _conn: Connection,
    video_stream: SendStream,
}

pub struct SenderInputSession {
    _conn: Connection,
    input_stream: Option<RecvStream>,
}

impl SenderSession {
    pub fn remote_address(&self) -> SocketAddr {
        self._conn.remote_address()
    }

    pub fn split(self) -> (SenderVideoSession, SenderInputSession) {
        let video_conn = self._conn.clone();
        (
            SenderVideoSession {
                _conn: video_conn,
                video_stream: self.video_stream,
            },
            SenderInputSession {
                _conn: self._conn,
                input_stream: self.input_stream,
            },
        )
    }

    /// 发送一个编码后的视频帧
    pub async fn send_video(
        &mut self,
        data: Vec<u8>,
        is_keyframe: bool,
        timestamp_us: u64,
    ) -> Result<(), TransportError> {
        send_video_on_stream(
            &mut self.video_stream,
            data,
            is_keyframe,
            timestamp_us,
            0,
            None,
        )
        .await
    }

    pub async fn send_video_with_trace(
        &mut self,
        data: Vec<u8>,
        is_keyframe: bool,
        timestamp_us: u64,
        frame_id: u64,
        trace: Option<VideoTrace>,
    ) -> Result<(), TransportError> {
        send_video_on_stream(
            &mut self.video_stream,
            data,
            is_keyframe,
            timestamp_us,
            frame_id,
            trace,
        )
        .await
    }

    /// 发送控制消息给 Viewer
    pub async fn send_control(&mut self, message: &ControlMessage) -> Result<(), TransportError> {
        send_control_on_stream(&mut self.video_stream, message).await
    }

    /// 仅接收输入事件（兼容 API）
    pub async fn recv_input(&mut self) -> Result<InputEvent, TransportError> {
        match self.recv_client_packet().await? {
            ClientPacket::Input(event) => Ok(event),
            ClientPacket::Control(_) => {
                Err(TransportError::InvalidPacketType(PacketType::Control as u8))
            }
        }
    }

    /// 接收客户端输入/控制消息
    pub async fn recv_client_packet(&mut self) -> Result<ClientPacket, TransportError> {
        self.ensure_input_stream().await?;
        recv_client_packet_from_stream(self.input_stream.as_mut().expect("stream initialized"))
            .await
    }

    async fn ensure_input_stream(&mut self) -> Result<(), TransportError> {
        if self.input_stream.is_none() {
            self.input_stream = Some(
                self._conn
                    .accept_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }
}

impl SenderVideoSession {
    /// 发送一个编码后的视频帧
    pub async fn send_video(
        &mut self,
        data: Vec<u8>,
        is_keyframe: bool,
        timestamp_us: u64,
    ) -> Result<(), TransportError> {
        send_video_on_stream(
            &mut self.video_stream,
            data,
            is_keyframe,
            timestamp_us,
            0,
            None,
        )
        .await
    }

    pub async fn send_video_with_trace(
        &mut self,
        data: Vec<u8>,
        is_keyframe: bool,
        timestamp_us: u64,
        frame_id: u64,
        trace: Option<VideoTrace>,
    ) -> Result<(), TransportError> {
        send_video_on_stream(
            &mut self.video_stream,
            data,
            is_keyframe,
            timestamp_us,
            frame_id,
            trace,
        )
        .await
    }

    /// 发送控制消息给 Viewer
    pub async fn send_control(&mut self, message: &ControlMessage) -> Result<(), TransportError> {
        send_control_on_stream(&mut self.video_stream, message).await
    }
}

impl SenderInputSession {
    /// 仅接收输入事件（兼容 API）
    pub async fn recv_input(&mut self) -> Result<InputEvent, TransportError> {
        match self.recv_client_packet().await? {
            ClientPacket::Input(event) => Ok(event),
            ClientPacket::Control(_) => {
                Err(TransportError::InvalidPacketType(PacketType::Control as u8))
            }
        }
    }

    /// 接收客户端输入/控制消息
    pub async fn recv_client_packet(&mut self) -> Result<ClientPacket, TransportError> {
        self.ensure_input_stream().await?;
        recv_client_packet_from_stream(self.input_stream.as_mut().expect("stream initialized"))
            .await
    }

    async fn ensure_input_stream(&mut self) -> Result<(), TransportError> {
        if self.input_stream.is_none() {
            self.input_stream = Some(
                self._conn
                    .accept_uni()
                    .await
                    .map_err(TransportError::Connection)?,
            );
        }
        Ok(())
    }
}

async fn send_video_on_stream(
    stream: &mut SendStream,
    data: Vec<u8>,
    is_keyframe: bool,
    timestamp_us: u64,
    frame_id: u64,
    trace: Option<VideoTrace>,
) -> Result<(), TransportError> {
    let payload = VideoPacket {
        timestamp_us,
        frame_id,
        trace,
        data,
    };
    let payload_bytes = payload.encode().map_err(TransportError::Serialize)?;

    let packet_type = if is_keyframe {
        PacketType::VideoKeyframe
    } else {
        PacketType::VideoFrame
    };

    let packet = Packet::new(packet_type, payload_bytes);
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

async fn recv_client_packet_from_stream(
    stream: &mut RecvStream,
) -> Result<ClientPacket, TransportError> {
    let mut header = [0u8; 5];
    stream
        .read_exact(&mut header)
        .await
        .map_err(TransportError::Read)?;

    let packet_type = header[0];
    let payload_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    let mut payload = vec![0u8; payload_len];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(TransportError::Read)?;

    match packet_type {
        x if x == PacketType::InputMouse as u8 || x == PacketType::InputKeyboard as u8 => {
            let event = InputEvent::decode(&payload).map_err(TransportError::Serialize)?;
            Ok(ClientPacket::Input(event))
        }
        x if x == PacketType::Control as u8 => {
            let message = ControlMessage::decode(&payload).map_err(TransportError::Serialize)?;
            Ok(ClientPacket::Control(message))
        }
        x if x == PacketType::Ping as u8 => Ok(ClientPacket::Control(ControlMessage::Ping)),
        x if x == PacketType::Pong as u8 => Ok(ClientPacket::Control(ControlMessage::Pong)),
        x => Err(TransportError::InvalidPacketType(x)),
    }
}
