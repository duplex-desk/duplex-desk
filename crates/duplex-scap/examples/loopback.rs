use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use duplex_codec::{VideoDecoder, VideoEncoder};
use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig, frame::DuplexScapFrame};

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
        match VideoEncoder::new(first_frame.width, first_frame.height, fps, 4000) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("create encoder failed: {err}");
                let _ = capturer.stop();
                return;
            }
        };

    let mut next_frame = Some(first_frame);
    let mut decoder: Option<VideoDecoder> = None;
    let mut decoded_rx: Option<Receiver<DuplexScapFrame>> = None;

    let mut count = 0usize;
    'main: for _ in 0..1200 {
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

        if !drain_packets(&packet_rx, &mut decoder, &mut decoded_rx, &mut capturer) {
            break;
        }

        if let Some(rx) = decoded_rx.as_ref() {
            loop {
                match rx.try_recv() {
                    Ok(frame) => {
                        println!(
                            "decoded #{}: {}x{} stride={} ts={}us",
                            count, frame.width, frame.height, frame.stride, frame.timestamp_us
                        );

                        if count < 3 {
                            save_as_png(&frame, &format!("frame_{count}.png"));
                        }

                        count += 1;
                        if count >= 30 {
                            break 'main;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'main,
                }
            }
        }
    }

    let _ = capturer.stop();
    println!("回环测试完成，请检查 frame_0.png ~ frame_2.png");
}

fn drain_packets(
    packet_rx: &Receiver<duplex_codec::EncodedPacket>,
    decoder: &mut Option<VideoDecoder>,
    decoded_rx: &mut Option<Receiver<DuplexScapFrame>>,
    capturer: &mut ScreenCapturer,
) -> bool {
    loop {
        match packet_rx.try_recv() {
            Ok(packet) => {
                if decoder.is_none() {
                    if !packet.is_keyframe {
                        continue;
                    }
                    let (new_decoder, rx) = match VideoDecoder::from_keyframe(&packet) {
                        Ok(v) => v,
                        Err(err) => {
                            eprintln!("create decoder failed: {err}");
                            let _ = capturer.stop();
                            return false;
                        }
                    };
                    if let Err(err) = new_decoder.decode(&packet) {
                        eprintln!("decode keyframe failed: {err}");
                        let _ = capturer.stop();
                        return false;
                    }
                    *decoder = Some(new_decoder);
                    *decoded_rx = Some(rx);
                    continue;
                }

                if let Some(decoder) = decoder.as_ref() {
                    if let Err(err) = decoder.decode(&packet) {
                        eprintln!("decode error: {err}");
                    }
                }
            }
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
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
