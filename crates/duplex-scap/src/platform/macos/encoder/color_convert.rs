use std::ffi::c_void;

// vImage is C API, so we bind it directly.
#[link(name = "Accelerate", kind = "framework")]
unsafe extern "C" {
    static kvImage_ARGBToYpCbCrMatrix_ITU_R_601_4: *const VImageARGBToYpCbCrMatrix;

    fn vImageConvert_ARGBToYpCbCr_GenerateConversion(
        matrix: *const VImageARGBToYpCbCrMatrix,
        pixel_range: *const VImageYpCbCrPixelRange,
        out_info: *mut VImageARGBToYpCbCr,
        in_argb_type: i32,
        out_ycbcr_type: i32,
        flags: u32,
    ) -> isize;

    fn vImageConvert_ARGB8888To420Yp8_CbCr8(
        src: *const VImageBuffer,
        dest_y: *mut VImageBuffer,
        dest_cbcr: *mut VImageBuffer,
        info: *const VImageARGBToYpCbCr,
        permute_map: *const u8,
        flags: u32,
    ) -> isize;
}

#[repr(C)]
struct VImageBuffer {
    data: *mut c_void,
    height: usize,
    width: usize,
    row_bytes: usize,
}

#[repr(C)]
struct VImageARGBToYpCbCrMatrix {
    r_yp: f32,
    g_yp: f32,
    b_yp: f32,
    r_cb: f32,
    g_cb: f32,
    b_cb_r_cr: f32,
    g_cr: f32,
    b_cr: f32,
}

#[repr(C, align(16))]
struct VImageARGBToYpCbCr {
    opaque: [u8; 128],
}

#[repr(C)]
struct VImageYpCbCrPixelRange {
    yp_bias: i32,
    cbcr_bias: i32,
    yp_range_max: i32,
    cbcr_range_max: i32,
    yp_max: i32,
    yp_min: i32,
    cbcr_max: i32,
    cbcr_min: i32,
}

const KV_IMAGE_ARGB8888: i32 = 0;
const KV_IMAGE_420YP8_CBCR8: i32 = 4;

pub struct NV12Frame {
    pub y_plane: Vec<u8>,
    pub uv_plane: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub y_stride: u32,
    pub uv_stride: u32,
}

/// BGRA (with stride padding) -> NV12.
pub fn bgra_to_nv12(
    bgra: &[u8],
    width: u32,
    height: u32,
    bgra_stride: u32,
) -> Result<NV12Frame, String> {
    if width == 0 || height == 0 {
        return Err("invalid frame size: width/height must be non-zero".to_string());
    }
    if (width & 1) != 0 || (height & 1) != 0 {
        return Err("NV12 requires even width and height".to_string());
    }

    let w = width as usize;
    let h = height as usize;
    let required_len = (bgra_stride as usize).saturating_mul(h);
    if bgra.len() < required_len {
        return Err(format!(
            "invalid BGRA buffer: len={} < required={}",
            bgra.len(),
            required_len
        ));
    }

    // Y plane: 1 byte per pixel, align stride to 64.
    let y_stride = ((w + 63) & !63) as u32;
    let uv_stride = y_stride; // Interleaved UV, same stride as Y.
    let uv_height = h / 2;

    let mut y_plane = vec![0u8; y_stride as usize * h];
    let mut uv_plane = vec![0u8; uv_stride as usize * uv_height];

    unsafe {
        let pixel_range = VImageYpCbCrPixelRange {
            yp_bias: 16,
            cbcr_bias: 128,
            yp_range_max: 235,
            cbcr_range_max: 240,
            yp_max: 235,
            yp_min: 16,
            cbcr_max: 240,
            cbcr_min: 16,
        };
        let mut info = VImageARGBToYpCbCr { opaque: [0; 128] };
        let info_status = vImageConvert_ARGBToYpCbCr_GenerateConversion(
            kvImage_ARGBToYpCbCrMatrix_ITU_R_601_4,
            &pixel_range,
            &mut info,
            KV_IMAGE_ARGB8888,
            KV_IMAGE_420YP8_CBCR8,
            0,
        );
        if info_status != 0 {
            return Err(format!(
                "vImageConvert_ARGBToYpCbCr_GenerateConversion failed: {}",
                info_status
            ));
        }

        let src = VImageBuffer {
            data: bgra.as_ptr() as *mut c_void,
            height: h,
            width: w,
            row_bytes: bgra_stride as usize,
        };
        let mut dst_y = VImageBuffer {
            data: y_plane.as_mut_ptr() as *mut c_void,
            height: h,
            width: w,
            row_bytes: y_stride as usize,
        };
        let mut dst_uv = VImageBuffer {
            data: uv_plane.as_mut_ptr() as *mut c_void,
            height: uv_height,
            width: w / 2,
            row_bytes: uv_stride as usize,
        };
        // Source is BGRA; tell ARGB API how to read channels.
        let permute_map = [3u8, 2u8, 1u8, 0u8];

        let status = vImageConvert_ARGB8888To420Yp8_CbCr8(
            &src,
            &mut dst_y,
            &mut dst_uv,
            &info,
            permute_map.as_ptr(),
            0,
        );
        if status != 0 {
            return Err(format!(
                "vImageConvert_ARGB8888To420Yp8_CbCr8 failed: {}",
                status
            ));
        }
    }

    Ok(NV12Frame {
        y_plane,
        uv_plane,
        width,
        height,
        y_stride,
        uv_stride,
    })
}
