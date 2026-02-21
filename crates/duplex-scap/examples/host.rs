use duplex_input::InputInjector;
use duplex_scap::{
    capturer::ScreenCapturer, config::DuplexScapConfig, encoder::VideoToolboxEncoder,
};
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
    let (encoder, packet_rx) = VideoToolboxEncoder::new(
        first_frame.width,
        first_frame.height,
        fps,
        4000, // 4 Mbps
    )
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

    let (packet_tx, mut packet_async_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(packet) = packet_rx.recv() {
            if packet_tx.send(packet).is_err() {
                break;
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
