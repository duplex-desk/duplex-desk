#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::InputInjector as PlatformInjector;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::InputInjector as PlatformInjector;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::InputInjector as PlatformInjector;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unsupported {
    use crate::{error::InputError, event::InputEvent};

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
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub use unsupported::InputInjector as PlatformInjector;
