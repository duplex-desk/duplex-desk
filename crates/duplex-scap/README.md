# duplex-scap

`duplex-scap` provides screen capture and video codec capabilities.

## Modules

- `capturer`: cross-platform `ScreenCapturer` entry
- `encoder`: encoder abstraction and macOS `VideoToolboxEncoder`
- `decoder`: decoder abstraction and macOS `VideoToolboxDecoder`
- `frame`: raw frame type `DuplexScapFrame`
- `config`: capture/display configuration (`DuplexScapConfig`)

## Current Status

- macOS:
  - ScreenCaptureKit capture
  - VideoToolbox H.264 encode/decode
- Windows/Linux: platform placeholders

## Examples

```bash
cargo run -p duplex-scap --example capture
cargo run -p duplex-scap --example encode
cargo run -p duplex-scap --example host
```

## Notes

- `EncodedPacket` can be converted to/from `duplex-proto::VideoPacket` for transport integration.
