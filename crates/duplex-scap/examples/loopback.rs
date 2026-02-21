use duplex_scap::{
    capturer::ScreenCapturer, config::DuplexScapConfig, decoder::VideoToolboxDecoder,
    encoder::VideoToolboxEncoder, frame::DuplexScapFrame,
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
    let (encoder, packet_rx) =
        VideoToolboxEncoder::new(first_frame.width, first_frame.height, fps, 4000)
            .expect("create encoder");
    encoder.encode(&first_frame).expect("encode first frame");

    // 采集 -> 编码
    std::thread::spawn(move || {
        while let Ok(frame) = frame_rx.recv() {
            if let Err(err) = encoder.encode(&frame) {
                eprintln!("encode error: {err}");
            }
        }
    });

    // 等第一个关键帧，初始化解码器
    let first_packet = packet_rx.recv().expect("receive first packet");
    assert!(first_packet.is_keyframe, "first packet must be keyframe");

    let (decoder, decoded_rx) =
        VideoToolboxDecoder::from_keyframe(&first_packet).expect("create decoder");
    decoder.decode(&first_packet).expect("decode first packet");

    // 编码 -> 解码
    std::thread::spawn(move || {
        while let Ok(packet) = packet_rx.recv() {
            if let Err(err) = decoder.decode(&packet) {
                eprintln!("decode error: {err}");
            }
        }
    });

    // 收解码后的帧，保存前 3 帧为 PNG 验证画面
    let mut count = 0usize;
    while let Ok(frame) = decoded_rx.recv() {
        println!(
            "decoded #{}: {}x{} stride={} ts={}us",
            count, frame.width, frame.height, frame.stride, frame.timestamp_us
        );

        if count < 3 {
            save_as_png(&frame, &format!("frame_{count}.png"));
        }

        count += 1;
        if count >= 30 {
            break;
        }
    }

    capturer.stop().expect("stop capture");
    println!("回环测试完成，请检查 frame_0.png ~ frame_2.png");
}

fn save_as_png(frame: &DuplexScapFrame, path: &str) {
    if frame.stride < frame.width.saturating_mul(4) {
        eprintln!(
            "skip save {path}: frame is not BGRA-like (stride={} width={})",
            frame.stride, frame.width
        );
        return;
    }

    let mut rgb = Vec::with_capacity((frame.width * frame.height * 3) as usize);
    for row in 0..frame.height as usize {
        let row_start = row * frame.stride as usize;
        for col in 0..frame.width as usize {
            let px = row_start + col * 4;
            // BGRA -> RGB
            rgb.push(frame.data[px + 2]); // R
            rgb.push(frame.data[px + 1]); // G
            rgb.push(frame.data[px]); // B
        }
    }

    image::save_buffer(
        path,
        &rgb,
        frame.width,
        frame.height,
        image::ColorType::Rgb8,
    )
    .expect("save png");
    println!("saved {path}");
}
