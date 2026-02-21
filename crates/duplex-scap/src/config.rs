#[derive(Debug, Clone)]
pub struct DuplexScapConfig {
    pub display_id: u32,
    pub fps: u64,
    pub pixel_format: PixelFormat,
}

impl Default for DuplexScapConfig {
    fn default() -> Self {
        Self {
            display_id: 0,
            fps: 30,
            pixel_format: PixelFormat::BGRA,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PixelFormat {
    BGRA,
}

impl PixelFormat {
    pub fn to_cv_pixel_format(&self) -> u32 {
        match self {
            // kCVPixelFormatType_32BGRA = 'BGRA' = 0x42475241
            PixelFormat::BGRA => 0x42475241,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DisplayInfo {
    pub display_id: u32,
    pub width: u32,
    pub height: u32,
}
