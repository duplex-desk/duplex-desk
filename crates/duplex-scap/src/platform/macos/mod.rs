use std::sync::{
    Arc, Condvar, Mutex,
    mpsc::{self, Receiver},
};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::{AnyThread, rc::Retained, runtime::ProtocolObject};
use objc2_core_media::CMTimeFlags;
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutputType,
};
use tracing::{debug, info, instrument, warn};

pub mod decoder;
pub mod encoder;
pub mod stream_output;

use self::stream_output::StreamOutput;
use crate::{
    config::{DisplayInfo, DuplexScapConfig},
    errors::DuplexScapError,
    frame::DuplexScapFrame,
};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    safe fn CGPreflightScreenCaptureAccess() -> bool;
    safe fn CGRequestScreenCaptureAccess();
}

pub struct MacOSCapturer {
    stream: Option<Retained<SCStream>>,
    output: Option<Retained<StreamOutput>>,
}

impl MacOSCapturer {
    pub fn new() -> Self {
        Self {
            stream: None,
            output: None,
        }
    }

    #[instrument(level = "info", skip(self, config))]
    pub fn start(
        &mut self,
        config: DuplexScapConfig,
    ) -> Result<Receiver<DuplexScapFrame>, DuplexScapError> {
        if self.stream.is_some() {
            return Err(DuplexScapError::AlreadyRunning);
        }

        if config.fps == 0 {
            warn!("fps is 0; this usually produces an invalid frame interval");
        }

        let content = Self::fetch_shareable_content()?;

        unsafe {
            // 1. Pick target display.
            let displays = content.displays();
            let display = if config.display_id == 0 {
                displays.iter().next()
            } else {
                displays.iter().find(|d| d.displayID() == config.display_id)
            }
            .ok_or(DuplexScapError::DisplayNotFound(config.display_id))?;

            // 2. Include all applications to avoid empty-frame behavior in ScreenCaptureKit.
            let apps = content.applications();
            let empty_windows: Retained<NSArray<_>> = NSArray::new();

            // 3. Build SCContentFilter.
            let filter = SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &apps,
                &empty_windows,
            );

            // 4. Build stream configuration.
            let stream_config = SCStreamConfiguration::new();
            stream_config.setWidth(display.width() as usize);
            stream_config.setHeight(display.height() as usize);
            stream_config.setPixelFormat(config.pixel_format.to_cv_pixel_format());
            stream_config.setMinimumFrameInterval(objc2_core_media::CMTime {
                value: 1,
                timescale: config.fps as i32,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            });
            stream_config.setShowsCursor(true);

            // 5. Build stream itself.
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &stream_config,
                None,
            );

            // 6. Channel + stream output delegate.
            let (tx, rx) = mpsc::sync_channel::<DuplexScapFrame>(1);
            let output = StreamOutput::new(tx);

            // 7. Register output callback queue.
            let queue = DispatchQueue::new("com.duplexscap.capture", DispatchQueueAttr::SERIAL);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Screen,
                    Some(&queue),
                )
                .map_err(|e| DuplexScapError::Internal(format!("{:?}", e)))?;

            // 8. Start capture and wait for completion callback.
            self.wait_start_capture(&stream)?;

            self.stream = Some(stream);
            self.output = Some(output);

            Ok(rx)
        }
    }

    pub fn stop(&mut self) -> Result<(), DuplexScapError> {
        let stream = match self.stream.take() {
            Some(s) => s,
            None => return Ok(()),
        };

        self.output = None;
        self.wait_stop_capture(&stream)
    }

    pub fn check_permissions() -> bool {
        let granted = CGPreflightScreenCaptureAccess();
        debug!(granted, "screen capture permission status checked");
        granted
    }

    pub fn request_permissions() {
        info!("requesting screen capture permission from macOS");
        CGRequestScreenCaptureAccess();
    }

    fn fetch_shareable_content() -> Result<Retained<SCShareableContent>, DuplexScapError> {
        let result: Arc<Mutex<Option<Result<Retained<SCShareableContent>, String>>>> =
            Arc::new(Mutex::new(None));
        let condvar = Arc::new(Condvar::new());

        let result_clone = Arc::clone(&result);
        let condvar_clone = Arc::clone(&condvar);

        let handler = RcBlock::new(
            move |content: *mut SCShareableContent, error: *mut NSError| {
                let value = if content.is_null() {
                    let msg = if error.is_null() {
                        "unknown error".to_string()
                    } else {
                        unsafe { format!("{:?}", (*error).localizedDescription()) }
                    };
                    tracing::error!(error_message = %msg, "shareable content callback failed");
                    Err(msg)
                } else {
                    debug!("shareable content callback succeeded");
                    // Convert raw pointer into Retained to safely keep ownership.
                    Ok(unsafe { Retained::retain(content).unwrap() })
                };

                *result_clone.lock().unwrap() = Some(value);
                condvar_clone.notify_one();
            },
        );

        debug!("requesting shareable content from ScreenCaptureKit");
        unsafe {
            SCShareableContent::getShareableContentWithCompletionHandler(&handler);
        }

        let mut lock = result.lock().unwrap();
        debug!("waiting for shareable content callback");
        while lock.is_none() {
            lock = condvar.wait(lock).unwrap();
        }

        let shareable_content = lock.take().unwrap().map_err(DuplexScapError::Internal)?;
        debug!("received shareable content");

        Ok(shareable_content)
    }

    #[instrument(level = "info")]
    pub fn list_displays() -> Result<Vec<DisplayInfo>, DuplexScapError> {
        let content = Self::fetch_shareable_content()?;

        let displays = unsafe {
            content
                .displays()
                .iter()
                .map(|d| DisplayInfo {
                    display_id: d.displayID(),
                    width: d.width() as u32,
                    height: d.height() as u32,
                })
                .collect()
        };
        Ok(displays)
    }

    fn wait_start_capture(&self, stream: &SCStream) -> Result<(), DuplexScapError> {
        self.wait_capture_op(|handler| unsafe {
            stream.startCaptureWithCompletionHandler(handler);
        })
    }

    fn wait_stop_capture(&self, stream: &SCStream) -> Result<(), DuplexScapError> {
        self.wait_capture_op(|handler| unsafe {
            stream.stopCaptureWithCompletionHandler(handler);
        })
    }

    fn wait_capture_op<F>(&self, op: F) -> Result<(), DuplexScapError>
    where
        F: FnOnce(Option<&block2::DynBlock<dyn Fn(*mut NSError)>>),
    {
        let result: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
        let condvar = Arc::new(Condvar::new());

        let result_c = Arc::clone(&result);
        let condvar_c = Arc::clone(&condvar);

        let handler = RcBlock::new(move |error: *mut NSError| {
            let value = if error.is_null() {
                Ok(())
            } else {
                Err(unsafe { format!("{:?}", (*error).localizedDescription()) })
            };

            *result_c.lock().unwrap() = Some(value);
            condvar_c.notify_one();
        });

        op(Some(&handler));

        let mut lock = result.lock().unwrap();
        while lock.is_none() {
            lock = condvar.wait(lock).unwrap();
        }

        lock.take().unwrap().map_err(DuplexScapError::Internal)
    }
}
