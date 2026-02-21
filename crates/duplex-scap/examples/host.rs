use std::sync::mpsc::{Receiver, TryRecvError};

use duplex_codec::{EncodedPacket, VideoEncoder};
use duplex_input::InputInjector;
use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig};
use duplex_transport::sender::Sender;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

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

    // 网络发送
    let sender = Sender::bind("0.0.0.0:5000".parse().expect("parse bind addr"))
        .await
        .expect("bind sender");

    println!("等待 Viewer 连接...");
    let session = sender.accept().await.expect("accept viewer");
    let (mut video_session, mut input_session) = session.split();
    println!("Viewer 已连接，开始推流");

    if !InputInjector::is_trusted() {
        eprintln!("当前未授予辅助功能权限，输入注入可能失败");
    }

    let injector = match InputInjector::new() {
        Ok(injector) => Some(injector),
        Err(err) => {
            eprintln!("input injector init failed: {err}");
            None
        }
    };

    let (packet_tx, mut packet_async_rx) = tokio::sync::mpsc::unbounded_channel::<EncodedPacket>();
    tokio::task::spawn_blocking(move || {
        let (encoder, packet_rx) =
            match VideoEncoder::new(first_frame.width, first_frame.height, fps, 4000) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("create encoder failed: {err}");
                    return;
                }
            };
        if let Err(err) = encoder.encode(&first_frame) {
            eprintln!("encode first frame failed: {err}");
            return;
        }
        if !drain_encoded_packets(&packet_rx, &packet_tx) {
            return;
        }

        while let Ok(frame) = frame_rx.recv() {
            if let Err(err) = encoder.encode(&frame) {
                eprintln!("encode error: {err}");
                continue;
            }
            if !drain_encoded_packets(&packet_rx, &packet_tx) {
                return;
            }
        }
    });

    loop {
        tokio::select! {
            maybe_packet = packet_async_rx.recv() => {
                let Some(packet) = maybe_packet else {
                    break;
                };

                if let Err(err) = video_session
                    .send_video(packet.data, packet.is_keyframe, packet.timestamp_us)
                    .await
                {
                    eprintln!("send error: {err}");
                    break;
                }
            }
            recv_result = input_session.recv_input() => {
                match recv_result {
                    Ok(event) => {
                        if let Some(injector) = injector.as_ref() {
                            if let Err(err) = injector.inject(&event) {
                                eprintln!("inject error: {err}");
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("recv input error: {err}");
                        break;
                    }
                }
            }
        }
    }

    let _ = capturer.stop();
}

fn drain_encoded_packets(
    packet_rx: &Receiver<EncodedPacket>,
    packet_tx: &tokio::sync::mpsc::UnboundedSender<EncodedPacket>,
) -> bool {
    loop {
        match packet_rx.try_recv() {
            Ok(packet) => {
                if packet_tx.send(packet).is_err() {
                    return false;
                }
            }
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}
