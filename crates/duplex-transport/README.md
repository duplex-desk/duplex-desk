# duplex-transport

`duplex-transport` provides QUIC-based transport channels for:

- Host -> Viewer: video frames + control messages
- Viewer -> Host: input events + control messages

## Core Types

- `Sender` / `SenderSession`: Host-side listener and sender
- `Receiver` / `ReceiverSession`: Viewer-side connector and receiver
- `ServerPacket` / `ClientPacket`: transport-layer packet envelopes

## Packet Format

All data uses a unified frame format:

- `PacketType` (1 byte)
- `len` (4 bytes, big-endian)
- `payload`

The `payload` is encoded with types from `duplex-proto`.

## Quick Example

```rust
// Host
let sender = duplex_transport::sender::Sender::bind("0.0.0.0:5000".parse()?).await?;
let mut session = sender.accept().await?;
session.send_video(vec![0; 1024], true, 0).await?;

// Viewer
let receiver = duplex_transport::receiver::Receiver::new()?;
let mut session = receiver.connect("127.0.0.1:5000".parse()?).await?;
let frame = session.recv_video().await?;
println!("{}", frame.data.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```
