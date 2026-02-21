use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),

    #[error("write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("read error: {0}")]
    Read(#[from] quinn::ReadExactError),

    #[error("tls error: {0}")]
    Tls(String),

    #[error("serialize error: {0}")]
    Serialize(String),

    #[error("invalid packet type: 0x{0:02x}")]
    InvalidPacketType(u8),

    #[error("endpoint closed")]
    EndpointClosed,
}
