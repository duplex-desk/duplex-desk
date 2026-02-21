use std::{collections::VecDeque, net::SocketAddr};

use duplex_codec::{EncodedPacket, VideoDecoder};
use duplex_input::InputEvent;
use duplex_proto::{ControlMessage, SessionState, VideoTrace};
use duplex_scap::frame::DuplexScapFrame;
use duplex_transport::{ServerPacket, receiver::Receiver};
use makepad_components::makepad_widgets::ToUISender;
use tokio::sync::{mpsc::UnboundedReceiver, watch};

use crate::task_event::{FrameTelemetry, TaskEvent, UiFrame};
use crate::time_utils::mono_now_us;

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

    let mut decoder: Option<VideoDecoder> = None;
    let mut decoded_rx: Option<std::sync::mpsc::Receiver<DuplexScapFrame>> = None;
    let mut input_open = true;
    let mut authorized = false;
    let mut pending_decode = VecDeque::<PendingDecodeTrace>::new();
    let mut latency_stats = ViewerLatencyStats::default();

    loop {
        tokio::select! {
            biased;
            _ = wait_stop(&mut stop_rx) => {
                let _ = input_session.send_control(&ControlMessage::Disconnect {
                    reason: "connection cancelled by viewer".to_string(),
                }).await;
                let _ = input_session.send_control(&ControlMessage::SessionState {
                    state: SessionState::Disconnected,
                }).await;
                return Ok(());
            },
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
                            ControlMessage::Disconnect { reason } => {
                                return Err(format!("peer canceled connection: {reason}"));
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

                            match VideoDecoder::from_keyframe(&key_packet) {
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

                        let viewer_recv_us = mono_now_us();
                        let frame_id = video_frame.frame_id;
                        let host_trace = video_frame.trace;

                        let packet = EncodedPacket {
                            data: video_frame.data,
                            is_keyframe: video_frame.is_keyframe,
                            timestamp_us: video_frame.timestamp_us,
                        };

                        let decode_submit_us = mono_now_us();
                        pending_decode.push_back(PendingDecodeTrace {
                            frame_id,
                            host_trace,
                            viewer_recv_us,
                            viewer_decode_submit_us: decode_submit_us,
                        });

                        if let Some(decoder) = decoder.as_ref()
                            && let Err(err) = decoder.decode(&packet) {
                                tracing::warn!("decode failed: {err}");
                                let _ = pending_decode.pop_back();
                                continue;
                            }

                        if let Some(rx) = decoded_rx.as_ref() {
                            let mut latest = None;
                            while let Ok(frame) = rx.try_recv() {
                                let viewer_decode_done_us = mono_now_us();
                                let telemetry = pending_decode
                                    .pop_front()
                                    .map(|pending| FrameTelemetry {
                                        frame_id: pending.frame_id,
                                        host_trace: pending.host_trace,
                                        viewer_recv_us: pending.viewer_recv_us,
                                        viewer_decode_submit_us: pending.viewer_decode_submit_us,
                                        viewer_decode_done_us,
                                    });

                                if let Some(t) = telemetry.as_ref() {
                                    latency_stats.observe(t);
                                }

                                latest = Some(UiFrame { frame, telemetry });
                            }

                            if let Some(frame_event) = latest {
                                if task_sender.send(TaskEvent::Frame(frame_event)).is_err() {
                                    return Ok(());
                                }
                                latency_stats.maybe_log();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct PendingDecodeTrace {
    frame_id: u64,
    host_trace: Option<VideoTrace>,
    viewer_recv_us: u64,
    viewer_decode_submit_us: u64,
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
struct ViewerLatencyStats {
    last_log_us: u64,
    frames: u64,
    host_capture_to_encode_submit: LatencyMetric,
    host_encode_submit_to_done: LatencyMetric,
    host_encode_done_to_send_submit: LatencyMetric,
    host_capture_to_send_submit: LatencyMetric,
    viewer_recv_to_decode_submit: LatencyMetric,
    viewer_decode_submit_to_done: LatencyMetric,
    viewer_recv_to_decode_done: LatencyMetric,
}

impl ViewerLatencyStats {
    fn observe(&mut self, telemetry: &FrameTelemetry) {
        self.frames = self.frames.saturating_add(1);

        if let Some(host) = telemetry.host_trace.as_ref() {
            if let Some(v) = host.host_encode_submit_us.checked_sub(host.host_capture_us) {
                self.host_capture_to_encode_submit.record(v);
            }
            if let Some(v) = host
                .host_encode_done_us
                .checked_sub(host.host_encode_submit_us)
            {
                self.host_encode_submit_to_done.record(v);
            }
            if let Some(v) = host
                .host_send_submit_us
                .checked_sub(host.host_encode_done_us)
            {
                self.host_encode_done_to_send_submit.record(v);
            }
            if let Some(v) = host.host_send_submit_us.checked_sub(host.host_capture_us) {
                self.host_capture_to_send_submit.record(v);
            }
        }

        if let Some(v) = telemetry
            .viewer_decode_submit_us
            .checked_sub(telemetry.viewer_recv_us)
        {
            self.viewer_recv_to_decode_submit.record(v);
        }
        if let Some(v) = telemetry
            .viewer_decode_done_us
            .checked_sub(telemetry.viewer_decode_submit_us)
        {
            self.viewer_decode_submit_to_done.record(v);
        }
        if let Some(v) = telemetry
            .viewer_decode_done_us
            .checked_sub(telemetry.viewer_recv_us)
        {
            self.viewer_recv_to_decode_done.record(v);
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
            "viewer_latency frames={} host(capture->enc_submit avg={}us max={}us, enc_submit->enc_done avg={}us max={}us, enc_done->send_submit avg={}us max={}us, capture->send_submit avg={}us max={}us) viewer(recv->dec_submit avg={}us max={}us, dec_submit->dec_done avg={}us max={}us, recv->dec_done avg={}us max={}us)",
            self.frames,
            self.host_capture_to_encode_submit.avg_us(),
            self.host_capture_to_encode_submit.max_us,
            self.host_encode_submit_to_done.avg_us(),
            self.host_encode_submit_to_done.max_us,
            self.host_encode_done_to_send_submit.avg_us(),
            self.host_encode_done_to_send_submit.max_us,
            self.host_capture_to_send_submit.avg_us(),
            self.host_capture_to_send_submit.max_us,
            self.viewer_recv_to_decode_submit.avg_us(),
            self.viewer_recv_to_decode_submit.max_us,
            self.viewer_decode_submit_to_done.avg_us(),
            self.viewer_decode_submit_to_done.max_us,
            self.viewer_recv_to_decode_done.avg_us(),
            self.viewer_recv_to_decode_done.max_us,
        );

        self.last_log_us = now_us;
        self.frames = 0;
        self.host_capture_to_encode_submit.reset();
        self.host_encode_submit_to_done.reset();
        self.host_encode_done_to_send_submit.reset();
        self.host_capture_to_send_submit.reset();
        self.viewer_recv_to_decode_submit.reset();
        self.viewer_decode_submit_to_done.reset();
        self.viewer_recv_to_decode_done.reset();
    }
}

async fn wait_stop(stop_rx: &mut watch::Receiver<bool>) {
    if *stop_rx.borrow() {
        return;
    }
    let _ = stop_rx.changed().await;
}
