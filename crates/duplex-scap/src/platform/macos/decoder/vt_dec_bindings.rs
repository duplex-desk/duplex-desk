#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::c_void;

use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags, CMVideoFormatDescription};
use objc2_core_video::CVPixelBuffer;

pub type OSStatus = i32;
pub type VTDecompressionSessionRef = *mut c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFTypeRef = *const c_void;

/// 解码输出回调
pub type VTDecompressionOutputCallback = unsafe extern "C" fn(
    decompressionOutputRefCon: *mut c_void,
    sourceFrameRefCon: *mut c_void,
    status: OSStatus,
    infoFlags: u32,
    imageBuffer: *mut CVPixelBuffer,
    presentationTimestamp: CMTime,
    presentationDuration: CMTime,
);

#[repr(C)]
pub struct VTDecompressionOutputCallbackRecord {
    pub decompressionOutputCallback: VTDecompressionOutputCallback,
    pub decompressionOutputRefCon: *mut c_void,
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    pub fn VTDecompressionSessionCreate(
        allocator: CFAllocatorRef,
        videoFormatDescription: *const CMVideoFormatDescription,
        videoDecoderSpecification: CFDictionaryRef,
        destinationImageBufferAttributes: CFDictionaryRef,
        outputCallback: *const VTDecompressionOutputCallbackRecord,
        decompressionSessionOut: *mut VTDecompressionSessionRef,
    ) -> OSStatus;

    pub fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sampleBuffer: *mut CMSampleBuffer,
        decodeFlags: u32,
        sourceFrameRefCon: *mut c_void,
        infoFlagsOut: *mut u32,
    ) -> OSStatus;

    pub fn VTDecompressionSessionWaitForAsynchronousFrames(
        session: VTDecompressionSessionRef,
    ) -> OSStatus;

    pub fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    /// 从 SPS + PPS 创建 CMVideoFormatDescription
    pub fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFAllocatorRef,
        parameterSetCount: usize,
        parameterSetPointers: *const *const u8,
        parameterSetSizes: *const usize,
        NALUnitHeaderLength: i32,
        formatDescriptionOut: *mut *const CMVideoFormatDescription,
    ) -> OSStatus;

    /// 把 Annex-B 的 NALU 数据包装成 CMSampleBuffer
    pub fn CMSampleBufferCreateReady(
        allocator: CFAllocatorRef,
        dataBuffer: *mut c_void, // CMBlockBufferRef
        formatDescription: *const CMVideoFormatDescription,
        numSamples: isize,
        numSampleTimingEntries: isize,
        sampleTimingArray: *const CMSampleTimingInfo,
        numSampleSizeEntries: isize,
        sampleSizeArray: *const usize,
        sampleBufferOut: *mut *mut CMSampleBuffer,
    ) -> OSStatus;

    pub fn CMBlockBufferCreateWithMemoryBlock(
        structureAllocator: CFAllocatorRef,
        memoryBlock: *mut c_void,
        blockLength: usize,
        blockAllocator: CFAllocatorRef,
        customBlockSource: *const c_void,
        offsetToData: usize,
        dataLength: usize,
        flags: u32,
        blockBufferOut: *mut *mut c_void,
    ) -> OSStatus;

    pub fn CMBlockBufferAssureBlockMemory(buffer: *mut c_void) -> OSStatus;

    pub fn CMBlockBufferReplaceDataBytes(
        sourceBytes: *const c_void,
        destinationBuffer: *mut c_void,
        offsetIntoDestination: usize,
        dataLength: usize,
    ) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFNumberCreate(
        allocator: CFAllocatorRef,
        theType: i32,
        valuePtr: *const c_void,
    ) -> CFTypeRef;

    pub fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const c_void,
        values: *const *const c_void,
        numValues: isize,
        keyCallBacks: *const c_void,
        valueCallBacks: *const c_void,
    ) -> CFDictionaryRef;

    pub fn CFRelease(cf: CFTypeRef);
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    pub static kCVPixelBufferPixelFormatTypeKey: *const c_void;
}

#[repr(C)]
pub struct CMSampleTimingInfo {
    pub duration: CMTime,
    pub presentationTimeStamp: CMTime,
    pub decodeTimeStamp: CMTime,
}

// kCMTimeInvalid
pub const CM_TIME_INVALID: CMTime = CMTime {
    value: 0,
    timescale: 0,
    flags: CMTimeFlags::empty(),
    epoch: 0,
};

pub const CF_NUMBER_SINT32_TYPE: i32 = 3;
