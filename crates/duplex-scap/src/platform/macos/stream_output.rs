use std::sync::mpsc::SyncSender;

use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVReturnSuccess,
};
use objc2_foundation::{NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{SCStream, SCStreamOutput, SCStreamOutputType};

use crate::frame::DuplexScapFrame;

pub struct StreamOutputIvars {
    sender: SyncSender<DuplexScapFrame>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DuplexScapStreamOutput"]
    #[ivars = StreamOutputIvars]
    pub struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output_sample_buffer_of_type(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            r#type: SCStreamOutputType,
        ) {
            if r#type != SCStreamOutputType::Screen {
                return;
            }

            if let Some(frame) = extract_frame(sample_buffer) {
                let _ = self.ivars().sender.try_send(frame);
            }
        }
    }
);

impl StreamOutput {
    pub fn new(sender: SyncSender<DuplexScapFrame>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(StreamOutputIvars { sender });
        let output: Retained<Self> = unsafe { msg_send![super(this), init] };
        output
    }
}

fn extract_frame(sample_buffer: &CMSampleBuffer) -> Option<DuplexScapFrame> {
    unsafe {
        // CMSampleBuffer -> CVImageBuffer (CVPixelBuffer type alias in CoreVideo).
        let image_buffer = sample_buffer.image_buffer()?;
        let pixel_buffer: &CVPixelBuffer = &image_buffer;

        // Lock pixel buffer memory for read-only access.
        let lock_flags = CVPixelBufferLockFlags::ReadOnly;
        if CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags) != kCVReturnSuccess {
            return None;
        }

        let width = CVPixelBufferGetWidth(pixel_buffer) as u32;
        let height = CVPixelBufferGetHeight(pixel_buffer) as u32;
        // Stride includes row alignment; it may be larger than width * 4.
        let stride = CVPixelBufferGetBytesPerRow(pixel_buffer) as u32;
        let base_ptr = CVPixelBufferGetBaseAddress(pixel_buffer) as *const u8;

        let frame = if base_ptr.is_null() || width == 0 || height == 0 {
            None
        } else {
            let byte_count = (stride as usize).checked_mul(height as usize)?;
            let data = std::slice::from_raw_parts(base_ptr, byte_count).to_vec();

            let timestamp_us = cm_time_to_micros(sample_buffer.presentation_time_stamp());

            Some(DuplexScapFrame {
                data,
                width,
                height,
                stride,
                timestamp_us,
            })
        };

        let _ = CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags);
        frame
    }
}

fn cm_time_to_micros(time: CMTime) -> u64 {
    if !time.flags.contains(CMTimeFlags::Valid) || time.timescale <= 0 || time.value <= 0 {
        return 0;
    }

    let micros = (time.value as i128) * 1_000_000i128 / (time.timescale as i128);
    if micros <= 0 {
        0
    } else if micros > u64::MAX as i128 {
        u64::MAX
    } else {
        micros as u64
    }
}
