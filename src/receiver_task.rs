use std::net::SocketAddr;

use duple_x_input::InputEvent;
use duple_x_scap::{decoder::VideoToolboxDecoder, encoder::EncodedPacket, frame::DuplexScapFrame};
use duplex_transport::receiver::Receiver;
use makepad_components::makepad_widgets::ToUISender;
use tokio::sync::mpsc::UnboundedReceiver;

pub type FrameSender = ToUISender<DuplexScapFrame>;
pub type InputReceiver = UnboundedReceiver<InputEvent>;

pub fn start_receiver_task(
    host_addr: SocketAddr,
    frame_sender: FrameSender,
    input_receiver: InputReceiver,
) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                tracing::error!("failed to create tokio runtime: {err}");
                return;
            }
        };

        runtime.block_on(async move {
            if let Err(err) = run_receiver(host_addr, frame_sender, input_receiver).await {
                tracing::error!("receiver task exited: {err}");
            }
        });
    });
}

async fn run_receiver(
    host_addr: SocketAddr,
    frame_sender: FrameSender,
    mut input_receiver: InputReceiver,
) -> Result<(), String> {
    let receiver = Receiver::new().map_err(|e| format!("receiver init failed: {e}"))?;
    let session = receiver
        .connect(host_addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let (mut video_session, mut input_session) = session.split();
    tracing::info!("connected to host {host_addr}");

    let mut decoder: Option<VideoToolboxDecoder> = None;
    let mut decoded_rx: Option<std::sync::mpsc::Receiver<DuplexScapFrame>> = None;
    let mut input_open = true;

    loop {
        tokio::select! {
            maybe_input = input_receiver.recv(), if input_open => {
                match maybe_input {
                    Some(event) => {
                        if let Err(err) = input_session.send_input(&event).await {
                            tracing::warn!("send_input failed: {err}");
                        }
                    }
                    None => {
                        input_open = false;
                        tracing::info!("input sender dropped");
                    }
                }
            }
            recv_result = video_session.recv_video() => {
                let video_frame = match recv_result {
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
    }
}
