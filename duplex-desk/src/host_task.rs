use std::net::SocketAddr;

use duplex_input::InputInjector;
use duplex_proto::{ControlMessage, SessionState};
use duplex_scap::{
    capturer::ScreenCapturer,
    config::DuplexScapConfig,
    encoder::{EncodedPacket, VideoToolboxEncoder},
};
use duplex_transport::{sender::Sender, ClientPacket};
use makepad_components::makepad_widgets::ToUISender;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    watch,
};

use crate::task_event::TaskEvent;

pub type TaskSender = ToUISender<TaskEvent>;

pub struct HostTaskHandle {
    stop_tx: watch::Sender<bool>,
    auth_tx: UnboundedSender<bool>,
}

impl HostTaskHandle {
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    pub fn approve(&self) {
        let _ = self.auth_tx.send(true);
    }

    pub fn reject(&self) {
        let _ = self.auth_tx.send(false);
    }
}

pub fn start_host_task(
    bind_addr: SocketAddr,
    device_code: String,
    task_sender: TaskSender,
) -> HostTaskHandle {
    let (stop_tx, stop_rx) = watch::channel(false);
    let (auth_tx, auth_rx) = mpsc::unbounded_channel::<bool>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                let _ = task_sender.send(TaskEvent::HostStopped(format!(
                    "failed to create host runtime: {err}"
                )));
                return;
            }
        };

        runtime.block_on(async move {
            let result = run_host(
                bind_addr,
                device_code,
                task_sender.clone(),
                auth_rx,
                stop_rx,
            )
            .await;
            let message = match result {
                Ok(()) => "host service stopped".to_string(),
                Err(err) => format!("host service error: {err}"),
            };
            let _ = task_sender.send(TaskEvent::HostStopped(message));
        });
    });

    HostTaskHandle { stop_tx, auth_tx }
}

async fn run_host(
    bind_addr: SocketAddr,
    device_code: String,
    task_sender: TaskSender,
    mut auth_rx: UnboundedReceiver<bool>,
    mut stop_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    let sender = Sender::bind(bind_addr)
        .await
        .map_err(|e| format!("failed to bind host sender: {e}"))?;
    let _ = task_sender.send(TaskEvent::HostStarted(bind_addr));

    if !InputInjector::is_trusted() {
        tracing::warn!("accessibility permission is not granted, input injection may fail");
    }
    let injector = InputInjector::new().ok();

    'accept_loop: loop {
        let session = tokio::select! {
            _ = wait_stop(&mut stop_rx) => {
                return Ok(());
            }
            accept_result = sender.accept() => {
                match accept_result {
                    Ok(session) => session,
                    Err(err) => return Err(format!("failed to accept viewer: {err}")),
                }
            }
        };

        let remote_addr = session.remote_address();
        let (mut video_session, mut input_session) = session.split();

        let _ = video_session
            .send_control(&ControlMessage::SessionState {
                state: SessionState::WaitingAuth,
            })
            .await;

        let (device_name, incoming_code) = loop {
            let packet = tokio::select! {
                _ = wait_stop(&mut stop_rx) => {
                    return Ok(());
                }
                recv_result = input_session.recv_client_packet() => {
                    match recv_result {
                        Ok(packet) => packet,
                        Err(err) => {
                            tracing::warn!("failed to read client packet: {err}");
                            continue 'accept_loop;
                        }
                    }
                }
            };

            match packet {
                ClientPacket::Control(ControlMessage::AuthRequest {
                    device_name,
                    device_code,
                }) => break (device_name, device_code),
                ClientPacket::Control(ControlMessage::Ping) => {
                    let _ = video_session.send_control(&ControlMessage::Pong).await;
                }
                ClientPacket::Input(_) | ClientPacket::Control(_) => {}
            }
        };

        if incoming_code != device_code {
            let _ = video_session
                .send_control(&ControlMessage::AuthDecision {
                    accepted: false,
                    reason: "device code mismatch".to_string(),
                })
                .await;
            let _ = video_session
                .send_control(&ControlMessage::SessionState {
                    state: SessionState::Rejected,
                })
                .await;
            continue;
        }

        while auth_rx.try_recv().is_ok() {}
        let _ = task_sender.send(TaskEvent::HostAwaitingApproval {
            remote_addr,
            device_name,
        });

        let approved = tokio::select! {
            _ = wait_stop(&mut stop_rx) => {
                return Ok(());
            }
            decision = auth_rx.recv() => decision.unwrap_or(false),
        };

        if !approved {
            let _ = video_session
                .send_control(&ControlMessage::AuthDecision {
                    accepted: false,
                    reason: "authorization denied by host".to_string(),
                })
                .await;
            let _ = video_session
                .send_control(&ControlMessage::SessionState {
                    state: SessionState::Rejected,
                })
                .await;
            continue;
        }

        // Start capture/encode only after local user approval.
        let (mut capturer, mut packet_async_rx) = match start_capture_pipeline() {
            Ok(v) => v,
            Err(err) => {
                let _ = video_session
                    .send_control(&ControlMessage::AuthDecision {
                        accepted: false,
                        reason: err.clone(),
                    })
                    .await;
                let _ = video_session
                    .send_control(&ControlMessage::SessionState {
                        state: SessionState::Rejected,
                    })
                    .await;
                continue 'accept_loop;
            }
        };

        let _ = video_session
            .send_control(&ControlMessage::AuthDecision {
                accepted: true,
                reason: "authorized".to_string(),
            })
            .await;
        let _ = video_session
            .send_control(&ControlMessage::SessionState {
                state: SessionState::Streaming,
            })
            .await;

        loop {
            tokio::select! {
                _ = wait_stop(&mut stop_rx) => {
                    let _ = video_session.send_control(&ControlMessage::SessionState { state: SessionState::Disconnected }).await;
                    let _ = capturer.stop();
                    return Ok(());
                }
                maybe_packet = packet_async_rx.recv() => {
                    let Some(packet) = maybe_packet else {
                        break;
                    };
                    if let Err(err) = video_session.send_video(packet.data, packet.is_keyframe, packet.timestamp_us).await {
                        tracing::warn!("video send failed: {err}");
                        break;
                    }
                }
                recv_result = input_session.recv_client_packet() => {
                    match recv_result {
                        Ok(ClientPacket::Input(event)) => {
                            if let Some(injector) = injector.as_ref() {
                                if let Err(err) = injector.inject(&event) {
                                    tracing::warn!("input inject failed: {err}");
                                }
                            }
                        }
                        Ok(ClientPacket::Control(ControlMessage::Ping)) => {
                            let _ = video_session.send_control(&ControlMessage::Pong).await;
                        }
                        Ok(ClientPacket::Control(_)) => {}
                        Err(err) => {
                            tracing::warn!("input receive failed: {err}");
                            break;
                        }
                    }
                }
            }
        }

        let _ = capturer.stop();
    }
}

async fn wait_stop(stop_rx: &mut watch::Receiver<bool>) {
    if *stop_rx.borrow() {
        return;
    }
    let _ = stop_rx.changed().await;
}

fn start_capture_pipeline() -> Result<(ScreenCapturer, tokio::sync::mpsc::Receiver<EncodedPacket>), String> {
    if !ScreenCapturer::check_permissions() {
        ScreenCapturer::request_permissions();
        return Err("screen recording permission is required".to_string());
    }

    let config = DuplexScapConfig::default();
    let fps = config.fps;

    let mut capturer = ScreenCapturer::new();
    let frame_rx = capturer
        .start(config)
        .map_err(|e| format!("failed to start capturer: {e}"))?;

    let first_frame = frame_rx
        .recv()
        .map_err(|e| format!("failed to receive first frame: {e}"))?;
    let (encoder, packet_rx) =
        VideoToolboxEncoder::new(first_frame.width, first_frame.height, fps, 4000)
            .map_err(|e| format!("failed to create encoder: {e}"))?;
    encoder
        .encode(&first_frame)
        .map_err(|e| format!("failed to encode first frame: {e}"))?;

    std::thread::spawn(move || {
        while let Ok(frame) = frame_rx.recv() {
            if let Err(err) = encoder.encode(&frame) {
                tracing::warn!("encode error: {err}");
            }
        }
    });

    let (packet_tx, packet_async_rx) = mpsc::channel(32);
    std::thread::spawn(move || {
        while let Ok(packet) = packet_rx.recv() {
            let _ = packet_tx.try_send(packet);
        }
    });

    Ok((capturer, packet_async_rx))
}
