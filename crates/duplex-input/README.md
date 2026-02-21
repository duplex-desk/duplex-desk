# duplex-input

`duplex-input` injects remote input events on the Host side.

## Public API

- `InputInjector`
  - `new()`: create an injector instance
  - `is_trusted()` (macOS): check Accessibility permission
  - `inject(&InputEvent)`: inject one input event

## Event Types

It reuses `duplex-proto` event definitions:
`InputEvent`, `Modifiers`, `MouseButton`, and `NormalizedPos`.

## Platform Support

- macOS: implemented (Core Graphics `CGEvent`)
- Windows: implemented (`SendInput`, normalized absolute cursor + keyboard/mouse injection)
- Linux: placeholder implementation returning `Unsupported`

## Typical Usage

```rust
use duplex_input::InputInjector;
use duplex_input::{InputEvent, NormalizedPos};

let injector = InputInjector::new()?;
injector.inject(&InputEvent::MouseMove {
    pos: NormalizedPos { x: 0.5, y: 0.5 },
})?;
# Ok::<(), duplex_input::InputError>(())
```
