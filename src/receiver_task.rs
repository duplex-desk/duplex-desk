use std::net::SocketAddr;

use duple_x_scap::{
    frame::DuplexScapFrame,
    platform::macos::{decoder::VideoToolboxDecoder, encoder::EncodedPacket},
};
use duplex_transport::receiver::Receiver;
use makepad_components::makepad_widgets::ToUISender;

pub type FrameSender = ToUISender<DuplexScapFrame>;

pub fn start_receiver_task(host_addr: SocketAddr, frame_sender: FrameSender) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                tracing::error!("failed to create tokio runtime: {err}");
                return;
            }
        };

        runtime.block_on(async move {
            if let Err(err) = run_receiver(host_addr, frame_sender).await {
                tracing::error!("receiver task exited: {err}");
            }
        });
    });
}

async fn run_receiver(host_addr: SocketAddr, frame_sender: FrameSender) -> Result<(), String> {
    let receiver = Receiver::new().map_err(|e| format!("receiver init failed: {e}"))?;
    let mut session = receiver
        .connect(host_addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    tracing::info!("connected to host {host_addr}");

    let mut decoder: Option<VideoToolboxDecoder> = None;
    let mut decoded_rx: Option<std::sync::mpsc::Receiver<DuplexScapFrame>> = None;

    loop {
        let video_frame = match session.recv_video().await {
            Ok(frame) => frame,
            Err(err) => return Err(format!("recv_video failed: {err}")),
        };

        if decoder.is_none() {
            if !video_frame.is_keyframe {
                continue;
            }

            let key_packet = EncodedPacket {
                data: video_frame.data.clone(),
                is_keyframe: true,
                timestamp_us: video_frame.timestamp_us,
            };

            match VideoToolboxDecoder::from_keyframe(&key_packet) {
                Ok((dec, rx)) => {
                    decoder = Some(dec);
                    decoded_rx = Some(rx);
                    tracing::info!("VideoToolbox decoder initialized");
                }
                Err(err) => {
                    tracing::error!("decoder initialization failed: {err}");
                    continue;
                }
            }
        }

        let packet = EncodedPacket {
            data: video_frame.data,
            is_keyframe: video_frame.is_keyframe,
            timestamp_us: video_frame.timestamp_us,
        };

        if let Some(decoder) = decoder.as_ref() {
            if let Err(err) = decoder.decode(&packet) {
                tracing::warn!("decode failed: {err}");
                continue;
            }
        }

        if let Some(rx) = decoded_rx.as_ref() {
            while let Ok(frame) = rx.try_recv() {
                if frame_sender.send(frame).is_err() {
                    return Ok(());
                }
            }
        }
    }
}
