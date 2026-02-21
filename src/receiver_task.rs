use std::net::SocketAddr;

use duple_x_input::InputEvent;
use duple_x_proto::{ControlMessage, SessionState};
use duple_x_scap::{decoder::VideoToolboxDecoder, encoder::EncodedPacket, frame::DuplexScapFrame};
use duplex_transport::{receiver::Receiver, ServerPacket};
use makepad_components::makepad_widgets::ToUISender;
use tokio::sync::{mpsc::UnboundedReceiver, watch};

use crate::task_event::TaskEvent;

pub type TaskSender = ToUISender<TaskEvent>;
pub type InputReceiver = UnboundedReceiver<InputEvent>;

pub struct ViewerTaskHandle {
    stop_tx: watch::Sender<bool>,
}

impl ViewerTaskHandle {
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

pub fn start_viewer_task(
    host_addr: SocketAddr,
    device_name: String,
    device_code: String,
    task_sender: TaskSender,
    input_receiver: InputReceiver,
) -> ViewerTaskHandle {
    let (stop_tx, stop_rx) = watch::channel(false);

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                let _ = task_sender.send(TaskEvent::ViewerStopped(format!(
                    "failed to create viewer runtime: {err}"
                )));
                return;
            }
        };

        runtime.block_on(async move {
            let result = run_viewer(
                host_addr,
                device_name,
                device_code,
                task_sender.clone(),
                input_receiver,
                stop_rx,
            )
            .await;
            let message = match result {
                Ok(()) => "viewer stopped".to_string(),
                Err(err) => format!("viewer error: {err}"),
            };
            let _ = task_sender.send(TaskEvent::ViewerStopped(message));
        });
    });

    ViewerTaskHandle { stop_tx }
}

async fn run_viewer(
    host_addr: SocketAddr,
    device_name: String,
    device_code: String,
    task_sender: TaskSender,
    mut input_receiver: InputReceiver,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    let receiver = Receiver::new().map_err(|e| format!("receiver init failed: {e}"))?;
    let session = receiver
        .connect(host_addr)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let (mut video_session, mut input_session) = session.split();
    let _ = task_sender.send(TaskEvent::ViewerConnected(host_addr));

    input_session
        .send_control(&ControlMessage::AuthRequest {
            device_name,
            device_code,
        })
        .await
        .map_err(|e| format!("failed to send auth request: {e}"))?;

    let mut decoder: Option<VideoToolboxDecoder> = None;
    let mut decoded_rx: Option<std::sync::mpsc::Receiver<DuplexScapFrame>> = None;
    let mut input_open = true;
    let mut authorized = false;

    loop {
        tokio::select! {
            _ = wait_stop(&mut stop_rx) => return Ok(()),
            maybe_input = input_receiver.recv(), if input_open && authorized => {
                match maybe_input {
                    Some(event) => {
                        if let Err(err) = input_session.send_input(&event).await {
                            tracing::warn!("send_input failed: {err}");
                        }
                    }
                    None => {
                        input_open = false;
                    }
                }
            }
            recv_result = video_session.recv_server_packet() => {
                let server_packet = match recv_result {
                    Ok(packet) => packet,
                    Err(err) => return Err(format!("recv packet failed: {err}")),
                };

                match server_packet {
                    ServerPacket::Control(control) => {
                        match control {
                            ControlMessage::AuthDecision { accepted, reason } => {
                                let _ = task_sender.send(TaskEvent::ViewerAuthResult {
                                    accepted,
                                    reason: reason.clone(),
                                });
                                if !accepted {
                                    return Err("authorization rejected by host".to_string());
                                }
                                authorized = true;
                            }
                            ControlMessage::SessionState { state } => {
                                if state == SessionState::Rejected || state == SessionState::Disconnected {
                                    return Err(format!("session ended by host: {state:?}"));
                                }
                            }
                            ControlMessage::Ping => {
                                let _ = input_session.send_control(&ControlMessage::Pong).await;
                            }
                            ControlMessage::Pong | ControlMessage::AuthRequest { .. } => {}
                        }
                    }
                    ServerPacket::Video(video_frame) => {
                        if !authorized {
                            continue;
                        }

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
                                if task_sender.send(TaskEvent::Frame(frame)).is_err() {
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn wait_stop(stop_rx: &mut watch::Receiver<bool>) {
    if *stop_rx.borrow() {
        return;
    }
    let _ = stop_rx.changed().await;
}
