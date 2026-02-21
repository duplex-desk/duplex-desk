mod vt_dec_bindings;

use std::ffi::c_void;
use std::ptr;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags, CMVideoFormatDescription};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
    kCVReturnSuccess,
};

use self::vt_dec_bindings::*;
use crate::frame::DuplexScapFrame;
use crate::platform::macos::encoder::EncodedPacket;

struct CallbackState {
    sender: SyncSender<DuplexScapFrame>,
}

pub struct VideoToolboxDecoder {
    session: VTDecompressionSessionRef,
    format_desc: *const CMVideoFormatDescription,
    _state: Arc<Mutex<CallbackState>>,
}

unsafe impl Send for VideoToolboxDecoder {}

impl VideoToolboxDecoder {
    /// 创建解码器，需要先拿到第一个关键帧（含 SPS+PPS）才能初始化
    pub fn new() -> (Self, Receiver<DuplexScapFrame>) {
        panic!("use VideoToolboxDecoder::from_keyframe() instead")
    }

    /// 从第一个关键帧（含 SPS+PPS）初始化解码器
    pub fn from_keyframe(
        packet: &EncodedPacket,
    ) -> Result<(Self, Receiver<DuplexScapFrame>), String> {
        if !packet.is_keyframe {
            return Err("first packet must be a keyframe with SPS/PPS".to_string());
        }

        // 1. 从 Annex-B 数据里解析出 SPS 和 PPS
        let (sps, pps) = extract_sps_pps(&packet.data)
            .ok_or_else(|| "failed to extract SPS/PPS from keyframe".to_string())?;

        // 2. 用 SPS+PPS 创建 CMVideoFormatDescription
        let format_desc = create_format_description(&sps, &pps)?;

        // 3. 创建 channel 和 callback state
        let (tx, rx) = mpsc::sync_channel::<DuplexScapFrame>(1);
        let state = Arc::new(Mutex::new(CallbackState { sender: tx }));
        let state_ptr = Arc::as_ptr(&state) as *mut c_void;

        // 4. 创建 VTDecompressionSession
        let callback_record = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: decode_output_callback,
            decompressionOutputRefCon: state_ptr,
        };

        // 强制解码输出为 BGRA，避免系统默认输出 NV12。
        let pixel_format = kCVPixelFormatType_32BGRA as i32;
        let pixel_format_num = unsafe {
            CFNumberCreate(
                ptr::null(),
                CF_NUMBER_SINT32_TYPE,
                &pixel_format as *const i32 as *const c_void,
            )
        };
        if pixel_format_num.is_null() {
            unsafe {
                CFRelease(format_desc as *const c_void);
            }
            return Err("CFNumberCreate(kCVPixelFormatType_32BGRA) failed".to_string());
        }

        let keys = [unsafe { kCVPixelBufferPixelFormatTypeKey }];
        let values = [pixel_format_num];
        let dest_attrs = unsafe {
            CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                ptr::null(),
                ptr::null(),
            )
        };
        if dest_attrs.is_null() {
            unsafe {
                CFRelease(pixel_format_num);
                CFRelease(format_desc as *const c_void);
            }
            return Err("CFDictionaryCreate(destinationImageBufferAttributes) failed".to_string());
        }

        let mut session: VTDecompressionSessionRef = ptr::null_mut();
        let status = unsafe {
            VTDecompressionSessionCreate(
                ptr::null(),
                format_desc,
                ptr::null(), // videoDecoderSpecification：让系统选最优
                dest_attrs,
                &callback_record,
                &mut session,
            )
        };
        unsafe {
            CFRelease(dest_attrs as *const c_void);
            CFRelease(pixel_format_num);
        }

        if status != 0 {
            unsafe {
                CFRelease(format_desc as *const c_void);
            }
            return Err(format!("VTDecompressionSessionCreate failed: {}", status));
        }

        Ok((
            Self {
                session,
                format_desc,
                _state: state,
            },
            rx,
        ))
    }

    /// 解码一帧
    pub fn decode(&self, packet: &EncodedPacket) -> Result<(), String> {
        // 把 Annex-B 转成 AVCC，再包装成 CMSampleBuffer
        let sample_buffer =
            annex_b_to_sample_buffer(&packet.data, self.format_desc, packet.timestamp_us)?;

        let status = unsafe {
            VTDecompressionSessionDecodeFrame(
                self.session,
                sample_buffer,
                0, // decode flags
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };

        unsafe {
            CFRelease(sample_buffer as *const c_void);
        }

        if status != 0 {
            return Err(format!(
                "VTDecompressionSessionDecodeFrame failed: {}",
                status
            ));
        }

        Ok(())
    }
}

impl Drop for VideoToolboxDecoder {
    fn drop(&mut self) {
        unsafe {
            let _ = VTDecompressionSessionWaitForAsynchronousFrames(self.session);
            VTDecompressionSessionInvalidate(self.session);
            if !self.format_desc.is_null() {
                CFRelease(self.format_desc as *const c_void);
            }
        }
    }
}

/// 解码输出回调：收到解码好的 CVPixelBuffer
unsafe extern "C" fn decode_output_callback(
    ref_con: *mut c_void,
    _source_frame_ref_con: *mut c_void,
    status: OSStatus,
    _info_flags: u32,
    image_buffer: *mut CVPixelBuffer,
    pts: CMTime,
    _duration: CMTime,
) {
    if status != 0 || image_buffer.is_null() || ref_con.is_null() {
        return;
    }

    let state = unsafe { &*(ref_con as *const Mutex<CallbackState>) };

    if let Some(frame) = unsafe { extract_decoded_frame(image_buffer, pts) } {
        if let Ok(guard) = state.lock() {
            let _ = guard.sender.try_send(frame);
        }
    }
}

/// 从解码后的 CVPixelBuffer 提取 Frame（和采集端逻辑一致）
unsafe fn extract_decoded_frame(
    pixel_buffer: *mut CVPixelBuffer,
    pts: CMTime,
) -> Option<DuplexScapFrame> {
    let pixel_buffer = unsafe { &*pixel_buffer };
    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    if unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags) } != kCVReturnSuccess {
        return None;
    }

    let width = CVPixelBufferGetWidth(pixel_buffer) as u32;
    let height = CVPixelBufferGetHeight(pixel_buffer) as u32;
    let stride = CVPixelBufferGetBytesPerRow(pixel_buffer) as u32;
    let base_ptr = CVPixelBufferGetBaseAddress(pixel_buffer) as *const u8;

    let frame = if base_ptr.is_null() || width == 0 || height == 0 {
        None
    } else {
        let byte_count = (stride as usize).checked_mul(height as usize)?;
        let data = unsafe { std::slice::from_raw_parts(base_ptr, byte_count) }.to_vec();

        let timestamp_us = if !pts.flags.contains(CMTimeFlags::Valid) || pts.timescale <= 0 {
            0
        } else {
            ((pts.value as i128) * 1_000_000i128 / (pts.timescale as i128))
                .clamp(0, u64::MAX as i128) as u64
        };

        Some(DuplexScapFrame {
            data,
            width,
            height,
            stride,
            timestamp_us,
        })
    };

    let _ = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags) };
    frame
}

/// 从 Annex-B 数据里找出 SPS 和 PPS NALU
fn extract_sps_pps(annex_b: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    let nalus = split_annex_b(annex_b);
    for nalu in nalus {
        if nalu.is_empty() {
            continue;
        }
        let nalu_type = nalu[0] & 0x1F;
        match nalu_type {
            7 => sps = Some(nalu.to_vec()),
            8 => pps = Some(nalu.to_vec()),
            _ => {}
        }
    }

    Some((sps?, pps?))
}

/// 把 Annex-B 按起始码切分成一个个 NALU（去掉起始码本身）
fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut nalus = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;

    while i + 3 <= data.len() {
        let is_start_code = (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1)
            || (i + 4 <= data.len()
                && data[i] == 0
                && data[i + 1] == 0
                && data[i + 2] == 0
                && data[i + 3] == 1);

        if is_start_code {
            if start < i {
                nalus.push(&data[start..i]);
            }
            start = if data[i + 2] == 1 { i + 3 } else { i + 4 };
            i = start;
        } else {
            i += 1;
        }
    }

    if start < data.len() {
        nalus.push(&data[start..]);
    }

    nalus
}

/// 用 SPS + PPS 创建 CMVideoFormatDescription
fn create_format_description(
    sps: &[u8],
    pps: &[u8],
) -> Result<*const CMVideoFormatDescription, String> {
    let param_sets: [*const u8; 2] = [sps.as_ptr(), pps.as_ptr()];
    let param_sizes: [usize; 2] = [sps.len(), pps.len()];
    let mut format_desc: *const CMVideoFormatDescription = ptr::null();

    let status = unsafe {
        CMVideoFormatDescriptionCreateFromH264ParameterSets(
            ptr::null(),
            2,
            param_sets.as_ptr(),
            param_sizes.as_ptr(),
            4, // AVCC 4-byte length prefix
            &mut format_desc,
        )
    };

    if status != 0 || format_desc.is_null() {
        Err(format!(
            "CMVideoFormatDescriptionCreateFromH264ParameterSets failed: {}",
            status
        ))
    } else {
        Ok(format_desc)
    }
}

/// Annex-B → AVCC，包装成 CMSampleBuffer 给解码器
fn annex_b_to_sample_buffer(
    annex_b: &[u8],
    format_desc: *const CMVideoFormatDescription,
    timestamp_us: u64,
) -> Result<*mut CMSampleBuffer, String> {
    let avcc = annex_b_to_avcc(annex_b);
    if avcc.is_empty() {
        return Err("annex_b_to_avcc produced empty payload".to_string());
    }

    unsafe {
        // 1. 创建 CMBlockBuffer（内部自分配内存），再写入 AVCC 数据
        let mut block_buffer: *mut c_void = ptr::null_mut();
        let create_status = CMBlockBufferCreateWithMemoryBlock(
            ptr::null(),
            ptr::null_mut(),
            avcc.len(),
            ptr::null(),
            ptr::null(),
            0,
            avcc.len(),
            0,
            &mut block_buffer,
        );
        if create_status != 0 || block_buffer.is_null() {
            return Err(format!(
                "CMBlockBufferCreateWithMemoryBlock failed: {}",
                create_status
            ));
        }

        let replace_status = CMBlockBufferReplaceDataBytes(
            avcc.as_ptr() as *const c_void,
            block_buffer,
            0,
            avcc.len(),
        );
        if replace_status != 0 {
            CFRelease(block_buffer as *const c_void);
            return Err(format!(
                "CMBlockBufferReplaceDataBytes failed: {}",
                replace_status
            ));
        }

        let assure_status = CMBlockBufferAssureBlockMemory(block_buffer);
        if assure_status != 0 {
            CFRelease(block_buffer as *const c_void);
            return Err(format!(
                "CMBlockBufferAssureBlockMemory failed: {}",
                assure_status
            ));
        }

        // 2. 时间戳
        let pts = CMTime {
            value: i64::try_from(timestamp_us).unwrap_or(i64::MAX),
            timescale: 1_000_000,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        };
        let timing = CMSampleTimingInfo {
            duration: CM_TIME_INVALID,
            presentationTimeStamp: pts,
            decodeTimeStamp: CM_TIME_INVALID,
        };
        let data_size = avcc.len();

        // 3. 创建 CMSampleBuffer
        let mut sample_buffer: *mut CMSampleBuffer = ptr::null_mut();
        let sample_status = CMSampleBufferCreateReady(
            ptr::null(),
            block_buffer,
            format_desc,
            1,
            1,
            &timing,
            1,
            &data_size,
            &mut sample_buffer,
        );

        // sample buffer retains block buffer on success; we can release local ref.
        CFRelease(block_buffer as *const c_void);

        if sample_status != 0 || sample_buffer.is_null() {
            Err(format!(
                "CMSampleBufferCreateReady failed: {}",
                sample_status
            ))
        } else {
            Ok(sample_buffer)
        }
    }
}

/// Annex-B 起始码 → AVCC 4 字节大端长度
fn annex_b_to_avcc(annex_b: &[u8]) -> Vec<u8> {
    let nalus = split_annex_b(annex_b);
    let mut avcc = Vec::new();

    for nalu in nalus {
        if nalu.is_empty() {
            continue;
        }
        // 跳过 SPS(7) 和 PPS(8)，解码器已从 format_desc 持有
        let nalu_type = nalu[0] & 0x1F;
        if nalu_type == 7 || nalu_type == 8 {
            continue;
        }

        let len = nalu.len() as u32;
        avcc.extend_from_slice(&len.to_be_bytes());
        avcc.extend_from_slice(nalu);
    }

    avcc
}
