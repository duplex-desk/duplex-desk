use std::net::SocketAddr;

use duplex_codec::{EncodedPacket, VideoEncoder};
use duplex_input::InputInjector;
use duplex_proto::{ControlMessage, SessionState, VideoTrace};
use duplex_scap::{capturer::ScreenCapturer, config::DuplexScapConfig, frame::DuplexScapFrame};
use duplex_transport::{ClientPacket, SenderVideoSession, sender::Sender};
use makepad_components::makepad_widgets::ToUISender;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    watch,
};

use crate::task_event::TaskEvent;
use crate::time_utils::mono_now_us;

pub type TaskSender = ToUISender<TaskEvent>;

pub struct HostTaskHandle {
    stop_tx: watch::Sender<bool>,
    auth_tx: UnboundedSender<bool>,
    disconnect_tx: UnboundedSender<()>,
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

    pub fn disconnect_session(&self) {
        let _ = self.disconnect_tx.send(());
    }
}

pub fn start_host_task(
    bind_addr: SocketAddr,
    device_code: String,
    task_sender: TaskSender,
) -> HostTaskHandle {
    let (stop_tx, stop_rx) = watch::channel(false);
    let (auth_tx, auth_rx) = mpsc::unbounded_channel::<bool>();
    let (disconnect_tx, disconnect_rx) = mpsc::unbounded_channel::<()>();

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
                disconnect_rx,
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

    HostTaskHandle {
        stop_tx,
        auth_tx,
        disconnect_tx,
    }
}

async fn run_host(
    bind_addr: SocketAddr,
    device_code: String,
    task_sender: TaskSender,
    mut auth_rx: UnboundedReceiver<bool>,
    mut disconnect_rx: UnboundedReceiver<()>,
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
        while auth_rx.try_recv().is_ok() {}
        while disconnect_rx.try_recv().is_ok() {}

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
                    send_disconnect(&mut video_session, "host stopped").await;
                    return Ok(());
                }
                disconnect = disconnect_rx.recv() => {
                    if disconnect.is_none() {
                        return Ok(());
                    }
                    send_disconnect(&mut video_session, "connection cancelled by host").await;
                    let _ = task_sender.send(TaskEvent::HostSessionEnded {
                        message: "Connection cancelled".to_string(),
                        peer_cancelled: false,
                    });
                    continue 'accept_loop;
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
                ClientPacket::Control(ControlMessage::Disconnect { reason }) => {
                    let _ = task_sender.send(TaskEvent::HostSessionEnded {
                        message: format!("Peer cancelled connection: {reason}"),
                        peer_cancelled: true,
                    });
                    continue 'accept_loop;
                }
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

        let approved = loop {
            tokio::select! {
                _ = wait_stop(&mut stop_rx) => {
                    send_disconnect(&mut video_session, "host stopped").await;
                    return Ok(());
                }
                disconnect = disconnect_rx.recv() => {
                    if disconnect.is_none() {
                        return Ok(());
                    }
                    send_disconnect(&mut video_session, "connection cancelled by host").await;
                    let _ = task_sender.send(TaskEvent::HostSessionEnded {
                        message: "Connection cancelled".to_string(),
                        peer_cancelled: false,
                    });
                    continue 'accept_loop;
                }
                decision = auth_rx.recv() => {
                    break decision.unwrap_or(false);
                }
                recv_result = input_session.recv_client_packet() => {
                    match recv_result {
                        Ok(ClientPacket::Control(ControlMessage::Disconnect { reason })) => {
                            let _ = task_sender.send(TaskEvent::HostSessionEnded {
                                message: format!("Peer cancelled connection: {reason}"),
                                peer_cancelled: true,
                            });
                            continue 'accept_loop;
                        }
                        Ok(ClientPacket::Control(ControlMessage::Ping)) => {
                            let _ = video_session.send_control(&ControlMessage::Pong).await;
                        }
                        Ok(_) => {}
                        Err(err) => {
                            tracing::warn!("viewer disconnected while awaiting approval: {err}");
                            let _ = task_sender.send(TaskEvent::HostSessionEnded {
                                message: "Viewer disconnected before approval".to_string(),
                                peer_cancelled: false,
                            });
                            continue 'accept_loop;
                        }
                    }
                }
            }
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
            let _ = task_sender.send(TaskEvent::HostSessionEnded {
                message: "Connection rejected".to_string(),
                peer_cancelled: false,
            });
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

        let mut latency_stats = HostLatencyStats::default();
        loop {
            tokio::select! {
                biased;
                _ = wait_stop(&mut stop_rx) => {
                    send_disconnect(&mut video_session, "host stopped").await;
                    let _ = capturer.stop();
                    return Ok(());
                }
                disconnect = disconnect_rx.recv() => {
                    if disconnect.is_none() {
                        let _ = capturer.stop();
                        return Ok(());
                    }
                    send_disconnect(&mut video_session, "connection cancelled by host").await;
                    let _ = task_sender.send(TaskEvent::HostSessionEnded {
                        message: "Connection cancelled".to_string(),
                        peer_cancelled: false,
                    });
                    let _ = capturer.stop();
                    continue 'accept_loop;
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
                        Ok(ClientPacket::Control(ControlMessage::Disconnect { reason })) => {
                            let _ = task_sender.send(TaskEvent::HostSessionEnded {
                                message: format!("Peer cancelled connection: {reason}"),
                                peer_cancelled: true,
                            });
                            break;
                        }
                        Ok(ClientPacket::Control(_)) => {}
                        Err(err) => {
                            tracing::warn!("input receive failed: {err}");
                            let _ = task_sender.send(TaskEvent::HostSessionEnded {
                                message: "Viewer disconnected".to_string(),
                                peer_cancelled: false,
                            });
                            break;
                        }
                    }
                }
                maybe_packet = packet_async_rx.recv() => {
                    let Some(mut packet) = maybe_packet else {
                        break;
                    };

                    let send_submit_us = mono_now_us();
                    packet.trace.host_send_submit_us = send_submit_us;
                    let result = video_session
                        .send_video_with_trace(
                            packet.packet.data,
                            packet.packet.is_keyframe,
                            packet.packet.timestamp_us,
                            packet.frame_id,
                            Some(packet.trace.clone()),
                        )
                        .await;
                    let send_done_us = mono_now_us();

                    if let Err(err) = result {
                        tracing::warn!("video send failed: {err}");
                        let _ = task_sender.send(TaskEvent::HostSessionEnded {
                            message: "Viewer disconnected".to_string(),
                            peer_cancelled: false,
                        });
                        break;
                    }

                    latency_stats.observe(&packet.trace, send_done_us);
                    latency_stats.maybe_log();
                }
            }
        }

        let _ = capturer.stop();
    }
}

#[derive(Debug)]
struct TracedEncodedPacket {
    packet: EncodedPacket,
    frame_id: u64,
    trace: VideoTrace,
}

#[derive(Debug)]
struct PendingEncodeTrace {
    frame_id: u64,
    host_capture_us: u64,
    host_encode_submit_us: u64,
}

#[derive(Default)]
struct LatencyMetric {
    count: u64,
    sum_us: u128,
    max_us: u64,
}

impl LatencyMetric {
    fn record(&mut self, value_us: u64) {
        self.count = self.count.saturating_add(1);
        self.sum_us = self.sum_us.saturating_add(value_us as u128);
        self.max_us = self.max_us.max(value_us);
    }

    fn avg_us(&self) -> u64 {
        if self.count == 0 {
            0
        } else {
            (self.sum_us / self.count as u128) as u64
        }
    }

    fn reset(&mut self) {
        self.count = 0;
        self.sum_us = 0;
        self.max_us = 0;
    }
}

#[derive(Default)]
struct HostLatencyStats {
    last_log_us: u64,
    frames: u64,
    capture_to_encode_submit: LatencyMetric,
    encode_submit_to_done: LatencyMetric,
    encode_done_to_send_submit: LatencyMetric,
    send_submit_to_send_done: LatencyMetric,
    capture_to_send_submit: LatencyMetric,
    capture_to_send_done: LatencyMetric,
}

impl HostLatencyStats {
    fn observe(&mut self, trace: &VideoTrace, send_done_us: u64) {
        self.frames = self.frames.saturating_add(1);

        if let Some(v) = trace
            .host_encode_submit_us
            .checked_sub(trace.host_capture_us)
        {
            self.capture_to_encode_submit.record(v);
        }
        if let Some(v) = trace
            .host_encode_done_us
            .checked_sub(trace.host_encode_submit_us)
        {
            self.encode_submit_to_done.record(v);
        }
        if let Some(v) = trace
            .host_send_submit_us
            .checked_sub(trace.host_encode_done_us)
        {
            self.encode_done_to_send_submit.record(v);
        }
        if let Some(v) = send_done_us.checked_sub(trace.host_send_submit_us) {
            self.send_submit_to_send_done.record(v);
        }
        if let Some(v) = trace.host_send_submit_us.checked_sub(trace.host_capture_us) {
            self.capture_to_send_submit.record(v);
        }
        if let Some(v) = send_done_us.checked_sub(trace.host_capture_us) {
            self.capture_to_send_done.record(v);
        }
    }

    fn maybe_log(&mut self) {
        if self.frames == 0 {
            return;
        }
        let now_us = mono_now_us();
        if now_us.saturating_sub(self.last_log_us) < 2_000_000 && self.frames < 120 {
            return;
        }

        tracing::info!(
            target: "duplex_desk_latency",
            "host_latency frames={} capture->enc_submit avg={}us max={}us, enc_submit->enc_done avg={}us max={}us, enc_done->send_submit avg={}us max={}us, send_submit->send_done avg={}us max={}us, capture->send_submit avg={}us max={}us, capture->send_done avg={}us max={}us",
            self.frames,
            self.capture_to_encode_submit.avg_us(),
            self.capture_to_encode_submit.max_us,
            self.encode_submit_to_done.avg_us(),
            self.encode_submit_to_done.max_us,
            self.encode_done_to_send_submit.avg_us(),
            self.encode_done_to_send_submit.max_us,
            self.send_submit_to_send_done.avg_us(),
            self.send_submit_to_send_done.max_us,
            self.capture_to_send_submit.avg_us(),
            self.capture_to_send_submit.max_us,
            self.capture_to_send_done.avg_us(),
            self.capture_to_send_done.max_us,
        );

        self.last_log_us = now_us;
        self.frames = 0;
        self.capture_to_encode_submit.reset();
        self.encode_submit_to_done.reset();
        self.encode_done_to_send_submit.reset();
        self.send_submit_to_send_done.reset();
        self.capture_to_send_submit.reset();
        self.capture_to_send_done.reset();
    }
}

async fn wait_stop(stop_rx: &mut watch::Receiver<bool>) {
    if *stop_rx.borrow() {
        return;
    }
    let _ = stop_rx.changed().await;
}

async fn send_disconnect(video_session: &mut SenderVideoSession, reason: &str) {
    let _ = video_session
        .send_control(&ControlMessage::Disconnect {
            reason: reason.to_string(),
        })
        .await;
    let _ = video_session
        .send_control(&ControlMessage::SessionState {
            state: SessionState::Disconnected,
        })
        .await;
}

fn start_capture_pipeline() -> Result<
    (
        ScreenCapturer,
        tokio::sync::mpsc::Receiver<TracedEncodedPacket>,
    ),
    String,
> {
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
    let (packet_tx, packet_async_rx) = mpsc::channel(1);

    std::thread::spawn(move || {
        let (encoder, packet_rx) =
            match VideoEncoder::new(first_frame.width, first_frame.height, fps, 4000) {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!("failed to create encoder: {err}");
                    return;
                }
            };

        let (trace_tx, trace_rx) = std::sync::mpsc::channel::<PendingEncodeTrace>();
        let packet_tx_forward = packet_tx.clone();
        std::thread::spawn(move || {
            while let Ok(packet) = packet_rx.recv() {
                let Ok(trace) = trace_rx.recv() else {
                    break;
                };

                let traced_packet = TracedEncodedPacket {
                    packet,
                    frame_id: trace.frame_id,
                    trace: VideoTrace {
                        host_capture_us: trace.host_capture_us,
                        host_encode_submit_us: trace.host_encode_submit_us,
                        host_encode_done_us: mono_now_us(),
                        host_send_submit_us: 0,
                    },
                };

                let _ = packet_tx_forward.try_send(traced_packet);
            }
        });

        let mut next_frame_id = 1u64;

        submit_frame_to_encoder(&encoder, &trace_tx, &mut next_frame_id, first_frame);

        while let Ok(frame) = frame_rx.recv() {
            submit_frame_to_encoder(&encoder, &trace_tx, &mut next_frame_id, frame);
        }
    });

    Ok((capturer, packet_async_rx))
}

fn submit_frame_to_encoder(
    encoder: &VideoEncoder,
    trace_tx: &std::sync::mpsc::Sender<PendingEncodeTrace>,
    next_frame_id: &mut u64,
    frame: DuplexScapFrame,
) {
    let frame_id = *next_frame_id;
    *next_frame_id = next_frame_id.saturating_add(1);

    let host_capture_us = mono_now_us();
    let host_encode_submit_us = mono_now_us();

    if let Err(err) = encoder.encode(&frame) {
        tracing::warn!("encode error: {err}");
        return;
    }

    if trace_tx
        .send(PendingEncodeTrace {
            frame_id,
            host_capture_us,
            host_encode_submit_us,
        })
        .is_err()
    {
        tracing::warn!("trace queue closed while submitting encoded frame");
    }
}
