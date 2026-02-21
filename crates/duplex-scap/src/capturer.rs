use std::sync::mpsc::Receiver;

use crate::{
    config::{DisplayInfo, DuplexScapConfig},
    errors::DuplexScapError,
    frame::DuplexScapFrame,
    platform::PlatformCapturer,
};

/// Cross-platform screen capturer entrypoint.
pub struct ScreenCapturer {
    inner: PlatformCapturer,
}

impl ScreenCapturer {
    pub fn new() -> Self {
        Self {
            inner: PlatformCapturer::new(),
        }
    }

    pub fn list_displays() -> Result<Vec<DisplayInfo>, DuplexScapError> {
        PlatformCapturer::list_displays()
    }

    pub fn start(
        &mut self,
        config: DuplexScapConfig,
    ) -> Result<Receiver<DuplexScapFrame>, DuplexScapError> {
        self.inner.start(config)
    }

    pub fn stop(&mut self) -> Result<(), DuplexScapError> {
        self.inner.stop()
    }

    pub fn check_permissions() -> bool {
        #[cfg(target_os = "macos")]
        {
            return PlatformCapturer::check_permissions();
        }

        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }

    pub fn request_permissions() {
        #[cfg(target_os = "macos")]
        {
            PlatformCapturer::request_permissions();
        }
    }
}

impl Default for ScreenCapturer {
    fn default() -> Self {
        Self::new()
    }
}

/// Backward-compatible alias for previous public API.
pub type DuplexScreenCapturer = ScreenCapturer;
