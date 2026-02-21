use duplex_scap::{
    capturer::ScreenCapturer, config::DuplexScapConfig, encoder::VideoToolboxEncoder,
};

fn main() {
    if !ScreenCapturer::check_permissions() {
        println!("需要屏幕录制权限");
        ScreenCapturer::request_permissions();
        return;
    }

    let config = DuplexScapConfig::default();
    let fps = config.fps;
    let mut capturer = ScreenCapturer::new();
    let frame_rx = capturer.start(config).expect("start capture");

    // 用首帧尺寸初始化编码器，避免分辨率不匹配。
    let first_frame = frame_rx.recv().expect("receive first frame");
    let (encoder, packet_rx) = VideoToolboxEncoder::new(
        first_frame.width,
        first_frame.height,
        fps,
        4000, // 4Mbps
    )
    .expect("create encoder");

    encoder.encode(&first_frame).expect("encode first frame");

    std::thread::spawn(move || {
        while let Ok(frame) = frame_rx.recv() {
            if let Err(err) = encoder.encode(&frame) {
                eprintln!("encode error: {err}");
            }
        }
    });

    let mut count = 0usize;
    while let Ok(packet) = packet_rx.recv() {
        println!(
            "packet #{}: {} bytes keyframe={} ts={}us",
            count,
            packet.data.len(),
            packet.is_keyframe,
            packet.timestamp_us
        );
        count += 1;
        if count >= 60 {
            break;
        }
    }

    capturer.stop().expect("stop capture");
}
