# duplex-desk

`duplex-desk` is a Rust workspace for remote desktop control.

It contains:

- `duplex-desk`: desktop application crate (Host/Viewer app)
- `duplex-scap`: screen capture and video encode/decode
- `duplex-transport`: QUIC transport layer
- `duplex-input`: remote input injection
- `duplex-proto`: shared protocol types

The current implementation runs in **single-direction control mode**: the app contains both Host and Viewer logic, but only one direction is active at a time.

## Workspace Layout

```
duplex-desk/
├── Cargo.toml                # workspace manifest
├── duplex-desk/              # app crate
│   ├── Cargo.toml
│   └── src/
└── crates/
    ├── duplex-proto/
    ├── duplex-input/
    ├── duplex-transport/
    └── duplex-scap/
```

## Development

```bash
cargo check --workspace
cargo run -p duplex-desk
```

## Notes

- UI is built with `makepad-components`.
- The current media/input path is primarily implemented for macOS (capture, codec, input injection).
