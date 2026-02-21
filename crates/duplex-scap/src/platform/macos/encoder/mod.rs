mod color_convert;
mod vt_bindings;

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use objc2_core_media::{
    CMFormatDescription, CMSampleBuffer, CMTime, CMTimeFlags,
    CMVideoFormatDescriptionGetH264ParameterSetAtIndex, kCMSampleAttachmentKey_NotSync,
};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreateWithPlanarBytes,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess,
};

use self::color_convert::{NV12Frame, bgra_to_nv12};
use self::vt_bindings::*;
use crate::frame::DuplexScapFrame;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryGetValue(the_dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFBooleanGetValue(boolean: *const c_void) -> u8;
}

/// Annex-B H.264 packet.
#[derive(Debug)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub is_keyframe: bool,
    pub timestamp_us: u64,
}

struct CallbackState {
    sender: SyncSender<EncodedPacket>,
}

pub struct VideoToolboxEncoder {
    session: VTCompressionSessionRef,
    // Keep callback state alive for the whole encoder lifetime.
    _state: Arc<Mutex<CallbackState>>,
    width: u32,
    height: u32,
}

// VTCompressionSessionRef is an opaque handle managed by VideoToolbox.
unsafe impl Send for VideoToolboxEncoder {}

impl VideoToolboxEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps: u64,
        bitrate_kbps: u32,
    ) -> Result<(Self, Receiver<EncodedPacket>), String> {
        if width == 0 || height == 0 || fps == 0 {
            return Err("width/height/fps must be non-zero".to_string());
        }

        let (tx, rx) = mpsc::sync_channel::<EncodedPacket>(10);
        let state = Arc::new(Mutex::new(CallbackState { sender: tx }));
        let state_ptr = Arc::as_ptr(&state) as *mut c_void;

        let mut session: VTCompressionSessionRef = ptr::null_mut();
        let status = unsafe {
            VTCompressionSessionCreate(
                ptr::null(),
                width as i32,
                height as i32,
                CM_VIDEO_CODEC_TYPE_H264,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                output_callback,
                state_ptr,
                &mut session,
            )
        };
        if status != 0 {
            return Err(format!("VTCompressionSessionCreate failed: {}", status));
        }

        if let Err(err) = unsafe { configure_session(session, fps, bitrate_kbps) } {
            unsafe {
                VTCompressionSessionInvalidate(session);
            }
            return Err(err);
        }

        Ok((
            Self {
                session,
                _state: state,
                width,
                height,
            },
            rx,
        ))
    }

    /// Encode one BGRA frame. Output is produced asynchronously via the channel.
    pub fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String> {
        if frame.width != self.width || frame.height != self.height {
            return Err(format!(
                "frame size mismatch: expected {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            ));
        }

        let nv12 = bgra_to_nv12(&frame.data, frame.width, frame.height, frame.stride)
            .map_err(|e| format!("bgra_to_nv12 failed: {e}"))?;
        let pixel_buffer = create_nv12_pixel_buffer(nv12)
            .ok_or_else(|| "failed to create CVPixelBuffer".to_string())?;

        let pts = CMTime {
            value: i64::try_from(frame.timestamp_us).unwrap_or(i64::MAX),
            timescale: 1_000_000,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        };
        // kCMTimeInvalid
        let duration = CMTime {
            value: 0,
            timescale: 0,
            flags: CMTimeFlags::empty(),
            epoch: 0,
        };

        let status = unsafe {
            VTCompressionSessionEncodeFrame(
                self.session,
                pixel_buffer,
                pts,
                duration,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };

        // Drop our local reference. VT keeps what it needs internally.
        unsafe { CFRelease(pixel_buffer.cast::<c_void>()) };

        if status != 0 {
            return Err(format!(
                "VTCompressionSessionEncodeFrame failed: {}",
                status
            ));
        }

        Ok(())
    }
}

impl Drop for VideoToolboxEncoder {
    fn drop(&mut self) {
        unsafe {
            let _ = VTCompressionSessionCompleteFrames(
                self.session,
                CMTime {
                    value: 0,
                    timescale: 0,
                    flags: CMTimeFlags::empty(),
                    epoch: 0,
                },
            );
            VTCompressionSessionInvalidate(self.session);
        }
    }
}

unsafe fn configure_session(
    session: VTCompressionSessionRef,
    fps: u64,
    bitrate_kbps: u32,
) -> Result<(), String> {
    unsafe fn set_property(
        session: VTCompressionSessionRef,
        key: *const c_void,
        value: *const c_void,
        name: &str,
    ) -> Result<(), String> {
        let status = unsafe { VTSessionSetProperty(session, key, value) };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("VTSessionSetProperty({name}) failed: {status}"))
        }
    }

    unsafe {
        set_property(
            session,
            kVTCompressionPropertyKey_RealTime,
            kCFBooleanTrue,
            "RealTime",
        )?;
        set_property(
            session,
            kVTCompressionPropertyKey_AllowFrameReordering,
            kCFBooleanFalse,
            "AllowFrameReordering",
        )?;
    }

    let bitrate = (bitrate_kbps.saturating_mul(1000)) as i64;
    let bitrate_ref = unsafe {
        CFNumberCreate(
            ptr::null(),
            CF_NUMBER_SINT64_TYPE,
            &bitrate as *const i64 as *const c_void,
        )
    };
    if bitrate_ref.is_null() {
        return Err("CFNumberCreate(AverageBitRate) failed".to_string());
    }
    let bitrate_result = unsafe {
        set_property(
            session,
            kVTCompressionPropertyKey_AverageBitRate,
            bitrate_ref,
            "AverageBitRate",
        )
    };
    unsafe { CFRelease(bitrate_ref) };
    bitrate_result?;

    let keyframe_interval = if fps > i32::MAX as u64 {
        i32::MAX
    } else {
        fps as i32
    };
    let keyframe_interval_ref = unsafe {
        CFNumberCreate(
            ptr::null(),
            CF_NUMBER_SINT32_TYPE,
            &keyframe_interval as *const i32 as *const c_void,
        )
    };
    if keyframe_interval_ref.is_null() {
        return Err("CFNumberCreate(MaxKeyFrameInterval) failed".to_string());
    }
    let keyframe_result = unsafe {
        set_property(
            session,
            kVTCompressionPropertyKey_MaxKeyFrameInterval,
            keyframe_interval_ref,
            "MaxKeyFrameInterval",
        )
    };
    unsafe { CFRelease(keyframe_interval_ref) };
    keyframe_result?;

    unsafe {
        set_property(
            session,
            kVTCompressionPropertyKey_ProfileLevel,
            kVTProfileLevel_H264_High_AutoLevel,
            "ProfileLevel",
        )?;
    }

    Ok(())
}

/// VTCompressionSession output callback.
unsafe extern "C" fn output_callback(
    output_callback_ref_con: *mut c_void,
    _source_frame_ref_con: *mut c_void,
    status: OSStatus,
    _info_flags: u32,
    sample_buffer: *mut CMSampleBuffer,
) {
    if status != 0 || sample_buffer.is_null() || output_callback_ref_con.is_null() {
        return;
    }

    let state = unsafe { &*(output_callback_ref_con as *const Mutex<CallbackState>) };
    let sample_buffer = unsafe { &*sample_buffer };

    if let Some(packet) = unsafe { extract_encoded_packet(sample_buffer) } {
        if let Ok(guard) = state.lock() {
            let _ = guard.sender.try_send(packet);
        }
    }
}

/// Convert encoded AVCC payload to Annex-B payload (+ SPS/PPS on keyframe).
unsafe fn extract_encoded_packet(sample_buffer: &CMSampleBuffer) -> Option<EncodedPacket> {
    let is_keyframe = unsafe { is_keyframe(sample_buffer) };

    let pts = unsafe { sample_buffer.presentation_time_stamp() };
    let timestamp_us = if pts.timescale <= 0 {
        0
    } else {
        ((pts.value as i128) * 1_000_000i128 / (pts.timescale as i128)).clamp(0, u64::MAX as i128)
            as u64
    };

    let mut annex_b = Vec::new();
    if is_keyframe {
        if let Some(format_desc) = unsafe { sample_buffer.format_description() } {
            unsafe {
                append_parameter_set(&mut annex_b, &format_desc, 0);
                append_parameter_set(&mut annex_b, &format_desc, 1);
            }
        }
    }

    let block_buffer = unsafe { sample_buffer.data_buffer()? };
    let data_length = unsafe { block_buffer.data_length() };

    let mut data_ptr: *mut i8 = ptr::null_mut();
    let pointer_status =
        unsafe { block_buffer.data_pointer(0, ptr::null_mut(), ptr::null_mut(), &mut data_ptr) };
    if pointer_status != 0 || data_ptr.is_null() || data_length < 4 {
        return None;
    }

    let data_ptr = data_ptr as *const u8;
    let mut offset = 0usize;
    while offset + 4 <= data_length {
        let nalu_len = u32::from_be_bytes([
            unsafe { *data_ptr.add(offset) },
            unsafe { *data_ptr.add(offset + 1) },
            unsafe { *data_ptr.add(offset + 2) },
            unsafe { *data_ptr.add(offset + 3) },
        ]) as usize;
        offset += 4;

        if nalu_len == 0 || offset.saturating_add(nalu_len) > data_length {
            break;
        }

        annex_b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        let nalu_data = unsafe { std::slice::from_raw_parts(data_ptr.add(offset), nalu_len) };
        annex_b.extend_from_slice(nalu_data);
        offset += nalu_len;
    }

    if annex_b.is_empty() {
        return None;
    }

    Some(EncodedPacket {
        data: annex_b,
        is_keyframe,
        timestamp_us,
    })
}

unsafe fn append_parameter_set(buf: &mut Vec<u8>, format_desc: &CMFormatDescription, index: usize) {
    let mut param_ptr: *const u8 = ptr::null();
    let mut param_len: usize = 0;
    let status = unsafe {
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            format_desc,
            index,
            &mut param_ptr,
            &mut param_len,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if status == 0 && !param_ptr.is_null() && param_len > 0 {
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        let param_slice = unsafe { std::slice::from_raw_parts(param_ptr, param_len) };
        buf.extend_from_slice(param_slice);
    }
}

unsafe extern "C-unwind" fn release_nv12_frame(
    release_ref_con: *mut c_void,
    _data_ptr: *const c_void,
    _data_size: usize,
    _number_of_planes: usize,
    _plane_addresses: *mut *const c_void,
) {
    if release_ref_con.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(release_ref_con as *mut NV12Frame));
    }
}

fn create_nv12_pixel_buffer(nv12: NV12Frame) -> Option<*mut CVPixelBuffer> {
    let nv12 = Box::new(nv12);
    let nv12_ptr = Box::into_raw(nv12);
    let nv12_ref = unsafe { &mut *nv12_ptr };

    let mut plane_base_addresses: [*mut c_void; 2] = [
        nv12_ref.y_plane.as_mut_ptr() as *mut c_void,
        nv12_ref.uv_plane.as_mut_ptr() as *mut c_void,
    ];
    let mut plane_widths: [usize; 2] = [nv12_ref.width as usize, nv12_ref.width as usize / 2];
    let mut plane_heights: [usize; 2] = [nv12_ref.height as usize, nv12_ref.height as usize / 2];
    let mut plane_strides: [usize; 2] = [nv12_ref.y_stride as usize, nv12_ref.uv_stride as usize];

    let mut pixel_buffer: *mut CVPixelBuffer = ptr::null_mut();
    let status = unsafe {
        CVPixelBufferCreateWithPlanarBytes(
            None,
            nv12_ref.width as usize,
            nv12_ref.height as usize,
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            ptr::null_mut(),
            0,
            2,
            NonNull::new(plane_base_addresses.as_mut_ptr())?,
            NonNull::new(plane_widths.as_mut_ptr())?,
            NonNull::new(plane_heights.as_mut_ptr())?,
            NonNull::new(plane_strides.as_mut_ptr())?,
            Some(release_nv12_frame),
            nv12_ptr as *mut c_void,
            None,
            NonNull::new(&mut pixel_buffer as *mut *mut CVPixelBuffer)?,
        )
    };

    if status != kCVReturnSuccess || pixel_buffer.is_null() {
        unsafe {
            drop(Box::from_raw(nv12_ptr));
        }
        return None;
    }

    Some(pixel_buffer)
}

unsafe fn is_keyframe(sample_buffer: &CMSampleBuffer) -> bool {
    let attachments = match unsafe { sample_buffer.sample_attachments_array(false) } {
        Some(a) => a,
        None => return true,
    };
    if attachments.count() <= 0 {
        return true;
    }

    let dict = unsafe { attachments.value_at_index(0) };
    if dict.is_null() {
        return true;
    }

    let not_sync = unsafe {
        CFDictionaryGetValue(
            dict,
            kCMSampleAttachmentKey_NotSync as *const _ as *const c_void,
        )
    };
    if not_sync.is_null() {
        return true;
    }

    unsafe { CFBooleanGetValue(not_sync) == 0 }
}
