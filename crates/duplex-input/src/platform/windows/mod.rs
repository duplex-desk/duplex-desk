use crate::error::InputError;
use crate::event::InputEvent;

pub struct InputInjector;

impl InputInjector {
    pub fn is_trusted() -> bool {
        false
    }

    pub fn new() -> Result<Self, InputError> {
        Err(InputError::Unsupported)
    }

    pub fn inject(&self, _event: &InputEvent) -> Result<(), InputError> {
        Err(InputError::Unsupported)
    }
}
