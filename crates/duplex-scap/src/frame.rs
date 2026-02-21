#[derive(Debug)]
pub struct DuplexScapFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub timestamp_us: u64,
}
