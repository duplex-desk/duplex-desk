pub mod error;
pub mod event;
mod platform;

pub use error::InputError;
pub use event::{InputEvent, Modifiers, MouseButton, NormalizedPos};
pub use platform::PlatformInjector as InputInjector;
