# duplex-proto

`duplex-proto` provides shared protocol data types and serialization utilities used across crates.

## Included Types

- `VideoPacket`: encoded video frame payload (Annex-B H.264)
- `InputEvent`: mouse/keyboard input events
- `ControlMessage`: auth, session state, and heartbeat control messages
- `SessionState`: session state enum

## Serialization

- Implemented with `serde + bincode(2)`.
- Each protocol type exposes `encode/decode` helpers.

## Typical Usage

```rust
use duplex_proto::{ControlMessage, SessionState};

let msg = ControlMessage::SessionState { state: SessionState::Streaming };
let bytes = msg.encode().unwrap();
let back = ControlMessage::decode(&bytes).unwrap();
```
