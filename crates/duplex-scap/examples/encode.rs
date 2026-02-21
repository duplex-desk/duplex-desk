use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use duplex_codec::VideoEncoder;
use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig};

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
    let (encoder, packet_rx) = match VideoEncoder::new(
        first_frame.width,
        first_frame.height,
        fps,
        4000, // 4Mbps
    ) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("create encoder failed: {err}");
            let _ = capturer.stop();
            return;
        }
    };

    let mut next_frame = Some(first_frame);
    let mut count = 0usize;
    'outer: for _ in 0..600 {
        let frame = if let Some(first) = next_frame.take() {
            first
        } else {
            match frame_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => frame,
                Err(_) => continue,
            }
        };

        if let Err(err) = encoder.encode(&frame) {
            eprintln!("encode error: {err}");
            continue;
        }

        loop {
            match packet_rx.try_recv() {
                Ok(packet) => {
                    println!(
                        "packet #{}: {} bytes keyframe={} ts={}us",
                        count,
                        packet.data.len(),
                        packet.is_keyframe,
                        packet.timestamp_us
                    );
                    count += 1;
                    if count >= 60 {
                        break 'outer;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break 'outer,
            }
        }
    }

    capturer.stop().expect("stop capture");
}
