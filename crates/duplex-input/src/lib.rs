pub mod error;
pub mod event;
mod platform;

pub use error::InputError;
pub use event::{InputEvent, Modifiers, MouseButton, NormalizedPos};

pub struct InputInjector {
    inner: platform::PlatformInjector,
}

impl InputInjector {
    pub fn is_trusted() -> bool {
        platform::PlatformInjector::is_trusted()
    }

    pub fn new() -> Result<Self, InputError> {
        Ok(Self {
            inner: platform::PlatformInjector::new()?,
        })
    }

    pub fn inject(&self, event: &InputEvent) -> Result<(), InputError> {
        self.inner.inject(event)
    }
}
