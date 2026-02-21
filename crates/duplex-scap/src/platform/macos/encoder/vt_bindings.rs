#![allow(non_snake_case, non_camel_case_types, dead_code)]

use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::CVPixelBuffer;
use std::ffi::c_void;

pub type OSStatus = i32;
pub type VTCompressionSessionRef = *mut c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CMVideoCodecType = u32;

pub const CM_VIDEO_CODEC_TYPE_H264: CMVideoCodecType = 0x61766331; // 'avc1'

pub type VTCompressionOutputCallback = unsafe extern "C" fn(
    outputCallbackRefCon: *mut c_void,
    sourceFrameRefCon: *mut c_void,
    status: OSStatus,
    infoFlags: u32,
    sampleBuffer: *mut CMSampleBuffer,
);

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    pub fn VTCompressionSessionCreate(
        allocator: CFAllocatorRef,
        width: i32,
        height: i32,
        codecType: CMVideoCodecType,
        encoderSpecification: CFDictionaryRef,
        sourceImageBufferAttributes: CFDictionaryRef,
        compressedDataAllocator: CFAllocatorRef,
        outputCallback: VTCompressionOutputCallback,
        outputCallbackRefCon: *mut c_void,
        compressionSessionOut: *mut VTCompressionSessionRef,
    ) -> OSStatus;

    pub fn VTCompressionSessionEncodeFrame(
        session: VTCompressionSessionRef,
        imageBuffer: *mut CVPixelBuffer,
        presentationTimestamp: CMTime,
        duration: CMTime,
        frameProperties: CFDictionaryRef,
        sourceFrameRefCon: *mut c_void,
        infoFlagsOut: *mut u32,
    ) -> OSStatus;

    pub fn VTCompressionSessionCompleteFrames(
        session: VTCompressionSessionRef,
        completeUntilPresentationTimeStamp: CMTime,
    ) -> OSStatus;

    pub fn VTCompressionSessionInvalidate(session: VTCompressionSessionRef);

    pub fn VTSessionSetProperty(
        session: VTCompressionSessionRef,
        propertyKey: *const c_void,   // CFStringRef
        propertyValue: *const c_void, // CFTypeRef
    ) -> OSStatus;
}

// Common property keys/constants exported by VideoToolbox.
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    pub static kVTCompressionPropertyKey_RealTime: *const c_void;
    pub static kVTCompressionPropertyKey_AllowFrameReordering: *const c_void;
    pub static kVTCompressionPropertyKey_AverageBitRate: *const c_void;
    pub static kVTCompressionPropertyKey_MaxKeyFrameInterval: *const c_void;
    pub static kVTCompressionPropertyKey_ProfileLevel: *const c_void;
    pub static kVTProfileLevel_H264_High_AutoLevel: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFBooleanTrue: *const c_void;
    pub static kCFBooleanFalse: *const c_void;
    pub fn CFNumberCreate(
        allocator: CFAllocatorRef,
        theType: i32,
        valuePtr: *const c_void,
    ) -> *const c_void;
    pub fn CFRelease(cf: *const c_void);
}

pub const CF_NUMBER_SINT32_TYPE: i32 = 3;
pub const CF_NUMBER_SINT64_TYPE: i32 = 4;
