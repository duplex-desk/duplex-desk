use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use duplex_codec::{EncodedPacket, VideoDecoder, VideoEncoder};
use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig, frame::DuplexScapFrame};
use image::ColorType;

fn main() {
    if !ScreenCapturer::check_permissions() {
        println!("screen capture permission required");
        ScreenCapturer::request_permissions();
        return;
    }

    let mut capturer = ScreenCapturer::new();
    let frame_rx = match capturer.start(DuplexScapConfig::default()) {
        Ok(rx) => rx,
        Err(err) => {
            eprintln!("start capture failed: {err}");
            return;
        }
    };

    let first_frame = match frame_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("receive first frame failed: {err}");
            let _ = capturer.stop();
            return;
        }
    };

    let (encoder, packet_rx) =
        match VideoEncoder::new(first_frame.width, first_frame.height, 30, 4000) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("create encoder failed: {err}");
                let _ = capturer.stop();
                return;
            }
        };
    if let Err(err) = encoder.encode(&first_frame) {
        eprintln!("encode first frame failed: {err}");
        let _ = capturer.stop();
        return;
    }

    let mut packets: Vec<EncodedPacket> = Vec::new();
    let collect_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < collect_deadline && packets.len() < 16 {
        match packet_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(packet) => packets.push(packet),
            Err(RecvTimeoutError::Timeout) => {
                if let Ok(frame) = frame_rx.recv_timeout(Duration::from_millis(20)) {
                    let _ = encoder.encode(&frame);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let key_packet = match packets.iter().find(|p| p.is_keyframe).cloned() {
        Some(packet) => packet,
        None => {
            eprintln!("no keyframe packet received from encoder");
            let _ = capturer.stop();
            return;
        }
    };
    let packets_count = packets.len();

    let (decoder, decoded_rx) = match VideoDecoder::from_keyframe(&key_packet) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("create decoder failed: {err}");
            let _ = capturer.stop();
            return;
        }
    };
    if let Err(err) = decoder.decode(&key_packet) {
        eprintln!("decode keyframe failed: {err}");
        let _ = capturer.stop();
        return;
    }
    for packet in &packets {
        let _ = decoder.decode(packet);
    }

    let output_png = "codec_loopback_capture.png";
    let mut decoded = 0usize;
    let mut png_saved = false;
    let mut last_frame: Option<DuplexScapFrame> = None;
    let decode_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < decode_deadline {
        match decoded_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                let avg_luma = avg_luma_bgra(&frame);
                println!(
                    "decoded: {}x{} stride={} ts={}us bytes={} avg_luma={:.1}",
                    frame.width,
                    frame.height,
                    frame.stride,
                    frame.timestamp_us,
                    frame.data.len(),
                    avg_luma,
                );
                decoded = decoded.saturating_add(1);
                if !png_saved && frame.timestamp_us > 0 && avg_luma > 2.0 {
                    match save_frame_png(&frame, Path::new(output_png)) {
                        Ok(()) => {
                            png_saved = true;
                            println!("saved decoded frame to {output_png}");
                        }
                        Err(err) => eprintln!("save png failed: {err}"),
                    }
                }
                last_frame = Some(frame);
                if decoded >= 3 {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    if !png_saved {
        if let Some(frame) = last_frame.as_ref() {
            if let Err(err) = save_frame_png(frame, Path::new(output_png)) {
                eprintln!("save fallback png failed: {err}");
            } else {
                png_saved = true;
                println!("saved fallback decoded frame to {output_png}");
            }
        }
    }

    let _ = capturer.stop();
    println!(
        "packets={} decoded={} png_saved={}",
        packets_count, decoded, png_saved
    );
}

fn save_frame_png(frame: &DuplexScapFrame, path: &Path) -> Result<(), String> {
    let rgba = bgra_to_rgba(frame)?;
    image::save_buffer(path, &rgba, frame.width, frame.height, ColorType::Rgba8)
        .map_err(|e| format!("image::save_buffer failed: {e}"))
}

fn bgra_to_rgba(frame: &DuplexScapFrame) -> Result<Vec<u8>, String> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = frame.stride as usize;
    let row = w.saturating_mul(4);
    if w == 0 || h == 0 || stride < row || frame.data.len() < stride.saturating_mul(h) {
        return Err("invalid BGRA frame".to_string());
    }

    let mut out = vec![0u8; row.saturating_mul(h)];
    for y in 0..h {
        let src = &frame.data[y * stride..y * stride + row];
        let dst = &mut out[y * row..(y + 1) * row];
        for x in 0..w {
            let si = x * 4;
            let di = si;
            dst[di] = src[si + 2];
            dst[di + 1] = src[si + 1];
            dst[di + 2] = src[si];
            dst[di + 3] = src[si + 3];
        }
    }
    Ok(out)
}

fn avg_luma_bgra(frame: &DuplexScapFrame) -> f32 {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = frame.stride as usize;
    let row = w.saturating_mul(4);
    if w == 0 || h == 0 || stride < row || frame.data.len() < stride.saturating_mul(h) {
        return 0.0;
    }

    let mut sum = 0f64;
    let mut count = 0usize;
    let step_y = (h / 180).max(1);
    let step_x = (w / 320).max(1);
    for y in (0..h).step_by(step_y) {
        let s = y * stride;
        for x in (0..w).step_by(step_x) {
            let i = s + x * 4;
            let b = frame.data[i] as f64;
            let g = frame.data[i + 1] as f64;
            let r = frame.data[i + 2] as f64;
            sum += 0.114 * b + 0.587 * g + 0.299 * r;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (sum / count as f64) as f32
    }
}
