use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use duplex_input::{InputEvent, Modifiers, MouseButton as InputMouseButton, NormalizedPos};
use makepad_components::button::MpButtonWidgetRefExt;
use makepad_components::makepad_widgets::*;
use makepad_components::modal::MpModalWidgetWidgetRefExt;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{
    host_task::{HostTaskHandle, start_host_task},
    receiver_task::{ViewerTaskHandle, start_viewer_task},
    task_event::{FrameTelemetry, TaskEvent, UiFrame},
    time_utils::mono_now_us,
    video_view::VideoFrameTexture,
};

const DEFAULT_HOST_BIND: &str = "0.0.0.0:5000";
const DEFAULT_VIEWER_TARGET: &str = "127.0.0.1:5000";

live_design! {
    use link::theme::*;
    use link::theme_colors::*;
    use link::shaders::*;
    use link::widgets::*;

    use makepad_components::button::*;
    use makepad_components::input::*;
    use makepad_components::modal::*;
    use makepad_components::tooltip::*;

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { title: "Duplex Desk" }

                body = <View> {
                    width: Fill,
                    height: Fill,
                    flow: Overlay,

                    main_content = <View> {
                        width: Fill,
                        height: Fill,
                        flow: Right,

                        video = <Image> {
                            width: Fill,
                            height: Fill,
                            fit: Stretch,
                        }

                        side_panel = <View> {
                            width: 360,
                            height: Fill,
                            flow: Down,
                            spacing: 10,
                            padding: { left: 12, right: 12, top: 12, bottom: 12 },
                            show_bg: true,
                            draw_bg: {
                                color: #0f172acc
                            }

                            panel_header = <View> {
                                width: Fill,
                                height: Fit,
                                flow: Right,
                                spacing: 8,
                                align: { x: 1.0, y: 0.5 }

                                panel_title = <Label> {
                                    width: Fill,
                                    draw_text: { color: #f8fafc, }
                                    text: "Connection Panel",
                                }

                                panel_toggle_btn = <MpButtonSecondary> {
                                    text: "<",
                                }
                            }

                            panel_content = <View> {
                                width: Fill,
                                height: Fill,
                                flow: Down,
                                spacing: 10,

                                mode_label = <Label> {
                                    draw_text: { color: #cbd5e1, }
                                    text: "Mode: IDLE",
                                }

                                status_text = <Label> {
                                    width: Fill,
                                    draw_text: { color: #94a3b8, wrap: Word }
                                    text: "Initializing...",
                                }

                                remote_label = <Label> {
                                    draw_text: { color: #94a3b8, }
                                    text: "Remote: -",
                                }

                                <View> {
                                    width: Fill,
                                    height: 1,
                                    show_bg: true,
                                    draw_bg: { color: #334155 }
                                }

                                settings_title = <Label> {
                                    draw_text: { color: #f8fafc, }
                                    text: "Settings",
                                }

                                host_bind_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "host bind address"
                                }

                                device_name_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "device name"
                                }

                                code_row = <View> {
                                    width: Fill,
                                    height: Fit,
                                    flow: Right,
                                    spacing: 6,
                                    align: { y: 0.5 },

                                    <Label> {
                                        draw_text: { color: #94a3b8, }
                                        text: "Device Code:"
                                    }

                                    device_code_value = <Label> {
                                        draw_text: { color: #f8fafc, }
                                        text: "------"
                                    }
                                }

                                fps_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "fps (next stage)"
                                }

                                bitrate_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "bitrate kbps (next stage)"
                                }

                                <View> {
                                    width: Fill,
                                    height: 1,
                                    show_bg: true,
                                    draw_bg: { color: #334155 }
                                }

                                viewer_title = <Label> {
                                    draw_text: { color: #f8fafc, }
                                    text: "Viewer Connect",
                                }

                                target_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "target ip:port"
                                }

                                viewer_code_input = <MpInput> {
                                    width: Fill,
                                    empty_text: "target device code"
                                }

                                btn_row = <View> {
                                    width: Fill,
                                    height: Fit,
                                    flow: Right,
                                    spacing: 8,

                                    connect_btn = <MpButtonPrimary> {
                                        text: "Connect",
                                    }

                                    host_btn = <MpButtonSecondary> {
                                        text: "Return Host",
                                    }

                                    cancel_btn = <MpButtonGhost> {
                                        text: "Cancel Connection",
                                    }
                                }
                            }
                        }
                    }
                    auth_modal = <MpModalWidget> {
                        content = {
                            dialog = <MpAlertDialog> {
                                width: 420,
                                header = {
                                    title = { text: "Authorize Remote Control" }
                                }
                                body = {
                                    <View> {
                                        width: Fill,
                                        flow: Down,
                                        spacing: 8,
                                        auth_remote_label = <Label> {
                                            draw_text: { color: #334155, }
                                            text: "Remote: -"
                                        }
                                        auth_device_label = <Label> {
                                            draw_text: { color: #334155, }
                                            text: "Device: -"
                                        }
                                        <Label> {
                                            draw_text: { color: #64748b, }
                                            text: "Allow this device to control your keyboard and mouse?"
                                        }
                                    }
                                }
                                footer = {
                                    auth_reject_btn = <MpButtonGhost> { text: "Reject" }
                                    auth_allow_btn = <MpButtonPrimary> { text: "Allow" }
                                }
                            }
                        }
                    }
                    notice_modal = <MpModalWidget> {
                        content = {
                            dialog = <MpAlertDialog> {
                                width: 420,
                                header = {
                                    title = { text: "Connection Notice" }
                                }
                                body = {
                                    <View> {
                                        width: Fill,
                                        flow: Down,
                                        spacing: 8,
                                        notice_message_label = <Label> {
                                            draw_text: { color: #334155, wrap: Word }
                                            text: "-"
                                        }
                                    }
                                }
                                footer = {
                                    notice_ok_btn = <MpButtonPrimary> { text: "OK" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Idle,
    Host,
    Viewer,
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
struct UiLatencyStats {
    last_log_us: u64,
    frames: u64,
    viewer_decode_done_to_ui_swap: LatencyMetric,
    viewer_recv_to_ui_swap: LatencyMetric,
    viewer_decode_submit_to_ui_swap: LatencyMetric,
}

impl UiLatencyStats {
    fn observe(&mut self, telemetry: &FrameTelemetry, ui_swap_us: u64) {
        self.frames = self.frames.saturating_add(1);

        if let Some(v) = ui_swap_us.checked_sub(telemetry.viewer_decode_done_us) {
            self.viewer_decode_done_to_ui_swap.record(v);
        }
        if let Some(v) = ui_swap_us.checked_sub(telemetry.viewer_recv_us) {
            self.viewer_recv_to_ui_swap.record(v);
        }
        if let Some(v) = ui_swap_us.checked_sub(telemetry.viewer_decode_submit_us) {
            self.viewer_decode_submit_to_ui_swap.record(v);
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
            "ui_latency frames={} decode_done->ui_swap avg={}us max={}us, recv->ui_swap avg={}us max={}us, decode_submit->ui_swap avg={}us max={}us",
            self.frames,
            self.viewer_decode_done_to_ui_swap.avg_us(),
            self.viewer_decode_done_to_ui_swap.max_us,
            self.viewer_recv_to_ui_swap.avg_us(),
            self.viewer_recv_to_ui_swap.max_us,
            self.viewer_decode_submit_to_ui_swap.avg_us(),
            self.viewer_decode_submit_to_ui_swap.max_us,
        );

        self.last_log_us = now_us;
        self.frames = 0;
        self.viewer_decode_done_to_ui_swap.reset();
        self.viewer_recv_to_ui_swap.reset();
        self.viewer_decode_submit_to_ui_swap.reset();
    }
}

#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust(ToUIReceiver::default())]
    task_rx: ToUIReceiver<TaskEvent>,
    #[rust(VideoFrameTexture::default())]
    video: VideoFrameTexture,
    #[rust(UiLatencyStats::default())]
    ui_latency: UiLatencyStats,
    #[rust(None)]
    host_handle: Option<HostTaskHandle>,
    #[rust(None)]
    viewer_handle: Option<ViewerTaskHandle>,
    #[rust(None)]
    input_tx: Option<UnboundedSender<InputEvent>>,
    #[rust(false)]
    input_capture_active: bool,
    #[rust(AppMode::Idle)]
    mode: AppMode,
    #[rust(DEFAULT_HOST_BIND.to_string())]
    host_bind: String,
    #[rust(default_device_name())]
    device_name: String,
    #[rust(generate_device_code())]
    device_code: String,
    #[rust(false)]
    viewer_authorized: bool,
    #[rust(false)]
    host_session_active: bool,
    #[rust(false)]
    panel_collapsed: bool,
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_components::makepad_widgets::live_design(cx);
        cx.link(live_id!(theme), live_id!(theme_desktop_light));
        cx.link(live_id!(theme_colors), live_id!(theme_colors_light));
        makepad_components::live_design(cx);
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        self.ui
            .text_input(ids!(host_bind_input))
            .set_text(cx, DEFAULT_HOST_BIND);
        self.ui
            .text_input(ids!(device_name_input))
            .set_text(cx, &self.device_name);
        self.ui
            .text_input(ids!(target_input))
            .set_text(cx, DEFAULT_VIEWER_TARGET);
        self.ui
            .text_input(ids!(viewer_code_input))
            .set_text(cx, &self.device_code);
        self.ui
            .label(ids!(device_code_value))
            .set_text(cx, &self.device_code);
        self.ui.text_input(ids!(fps_input)).set_text(cx, "30");
        self.ui.text_input(ids!(bitrate_input)).set_text(cx, "4000");

        self.mode = AppMode::Idle;
        self.host_session_active = false;
        self.update_mode_label(cx);
        self.set_remote(cx, "-");
        self.set_status(
            cx,
            "Idle. Connect as viewer or click Return Host to accept requests.",
        );
        self.clear_video(cx);
        self.set_side_panel_collapsed(cx, false);
        self.update_action_buttons(cx);
    }

    fn handle_signal(&mut self, cx: &mut Cx) {
        let mut latest_frame: Option<UiFrame> = None;

        while let Ok(event) = self.task_rx.try_recv() {
            match event {
                TaskEvent::Frame(frame) => {
                    latest_frame = Some(frame);
                }
                TaskEvent::HostStarted(addr) => {
                    if self.mode == AppMode::Host {
                        self.set_status(
                            cx,
                            &format!("Host listener ready: {addr} (capture starts after approval)"),
                        );
                    }
                }
                TaskEvent::HostAwaitingApproval {
                    remote_addr,
                    device_name,
                } => {
                    self.host_session_active = true;
                    self.ui
                        .label(ids!(auth_remote_label))
                        .set_text(cx, &format!("Remote: {remote_addr}"));
                    self.ui
                        .label(ids!(auth_device_label))
                        .set_text(cx, &format!("Device: {device_name}"));
                    self.ui.mp_modal_widget(ids!(auth_modal)).open(cx);
                    self.set_remote(cx, &remote_addr.to_string());
                    self.set_status(cx, "Incoming control request");
                    self.update_action_buttons(cx);
                }
                TaskEvent::HostSessionEnded {
                    message,
                    peer_cancelled,
                } => {
                    if self.mode == AppMode::Host {
                        self.host_session_active = false;
                        self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
                        self.set_remote(cx, "-");
                        self.set_status(cx, &format!("Host ready. {message}"));
                        self.clear_video(cx);
                        self.update_action_buttons(cx);
                        if peer_cancelled {
                            self.show_notice(cx, "The remote side cancelled the connection.");
                        }
                    }
                }
                TaskEvent::HostStopped(message) => {
                    if self.mode == AppMode::Host {
                        self.host_handle = None;
                        self.host_session_active = false;
                        self.mode = AppMode::Idle;
                        self.update_mode_label(cx);
                        self.set_remote(cx, "-");
                        self.set_status(cx, &message);
                        self.clear_video(cx);
                        self.update_action_buttons(cx);
                    }
                }
                TaskEvent::ViewerConnected(addr) => {
                    if self.mode == AppMode::Viewer {
                        self.set_remote(cx, &addr.to_string());
                        self.set_status(
                            cx,
                            &format!(
                                "Connected to {addr}. Waiting for the controlled computer to approve..."
                            ),
                        );
                        self.update_action_buttons(cx);
                    }
                }
                TaskEvent::ViewerAuthResult { accepted, reason } => {
                    if self.mode == AppMode::Viewer {
                        self.viewer_authorized = accepted;
                        if accepted {
                            self.set_status(cx, &format!("Connected: {reason}"));
                        } else {
                            self.set_status(cx, &format!("Viewer rejected: {reason}"));
                        }
                        self.update_action_buttons(cx);
                    }
                }
                TaskEvent::ViewerStopped(message) => {
                    if self.mode == AppMode::Viewer {
                        self.viewer_handle = None;
                        self.input_tx = None;
                        self.viewer_authorized = false;
                        self.mode = AppMode::Idle;
                        self.update_mode_label(cx);
                        self.set_remote(cx, "-");
                        self.set_status(cx, &message);
                        self.clear_video(cx);
                        self.update_action_buttons(cx);
                        if is_peer_cancel_message(&message) {
                            self.show_notice(cx, "The remote side cancelled the connection.");
                        }
                    }
                }
            }
        }

        if let Some(frame_event) = latest_frame {
            if self.video.update_frame(
                cx,
                &frame_event.frame.data,
                frame_event.frame.width as usize,
                frame_event.frame.height as usize,
                frame_event.frame.stride as usize,
            ) {
                let image = self.ui.image(ids!(video));
                image.set_texture(cx, self.video.texture());
                image.redraw(cx);

                if let Some(telemetry) = frame_event.telemetry.as_ref() {
                    let ui_swap_us = mono_now_us();
                    self.ui_latency.observe(telemetry, ui_swap_us);
                    self.ui_latency.maybe_log();
                }
            }
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        if self.mode != AppMode::Viewer && self.ui.mp_button(ids!(connect_btn)).clicked(actions) {
            let target = self.ui.text_input(ids!(target_input)).text();
            let target = target.trim();
            if target.is_empty() {
                self.set_status(cx, "Viewer target is empty");
                return;
            }

            let host_addr: SocketAddr = match target.parse() {
                Ok(addr) => addr,
                Err(err) => {
                    self.set_status(cx, &format!("Invalid target address: {err}"));
                    return;
                }
            };

            self.start_viewer_mode(cx, host_addr);
        }

        if self.ui.mp_button(ids!(panel_toggle_btn)).clicked(actions) {
            self.set_side_panel_collapsed(cx, !self.panel_collapsed);
        }

        if self.mode != AppMode::Host && self.ui.mp_button(ids!(host_btn)).clicked(actions) {
            self.start_host_mode(cx);
        }

        if self.ui.mp_button(ids!(cancel_btn)).clicked(actions) {
            self.cancel_current_connection(cx);
        }

        if self.ui.mp_button(ids!(auth_allow_btn)).clicked(actions) {
            if let Some(handle) = self.host_handle.as_ref() {
                handle.approve();
            }
            self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
            self.set_status(cx, "Auth approved");
            self.host_session_active = true;
            self.update_action_buttons(cx);
        }

        if self.ui.mp_button(ids!(auth_reject_btn)).clicked(actions) {
            if let Some(handle) = self.host_handle.as_ref() {
                handle.reject();
            }
            self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
            self.set_status(cx, "Auth rejected");
            self.host_session_active = false;
            self.set_remote(cx, "-");
            self.update_action_buttons(cx);
        }

        if self
            .ui
            .mp_modal_widget(ids!(auth_modal))
            .close_requested(actions)
        {
            if let Some(handle) = self.host_handle.as_ref() {
                handle.reject();
            }
            self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
            self.set_status(cx, "Auth dismissed");
            self.host_session_active = false;
            self.set_remote(cx, "-");
            self.update_action_buttons(cx);
        }

        if self.ui.mp_button(ids!(notice_ok_btn)).clicked(actions) {
            self.ui.mp_modal_widget(ids!(notice_modal)).close(cx);
        }

        if self
            .ui
            .mp_modal_widget(ids!(notice_modal))
            .close_requested(actions)
        {
            self.ui.mp_modal_widget(ids!(notice_modal)).close(cx);
        }
    }
}

impl App {
    fn to_modifiers(modifiers: &KeyModifiers) -> Modifiers {
        Modifiers {
            shift: modifiers.shift,
            ctrl: modifiers.control,
            alt: modifiers.alt,
            meta: modifiers.logo,
        }
    }

    fn map_mouse_button(
        button: makepad_components::makepad_widgets::MouseButton,
    ) -> Option<InputMouseButton> {
        if button.is_primary() {
            Some(InputMouseButton::Left)
        } else if button.is_secondary() {
            Some(InputMouseButton::Right)
        } else if button.is_middle() {
            Some(InputMouseButton::Middle)
        } else {
            None
        }
    }

    fn set_status(&mut self, cx: &mut Cx, text: &str) {
        self.ui.label(ids!(status_text)).set_text(cx, text);
    }

    fn set_remote(&mut self, cx: &mut Cx, remote: &str) {
        self.ui
            .label(ids!(remote_label))
            .set_text(cx, &format!("Remote: {remote}"));
    }

    fn show_notice(&mut self, cx: &mut Cx, message: &str) {
        self.ui
            .label(ids!(notice_message_label))
            .set_text(cx, message);
        self.ui.mp_modal_widget(ids!(notice_modal)).open(cx);
    }

    fn clear_video(&mut self, cx: &mut Cx) {
        self.video.clear();
        let image = self.ui.image(ids!(video));
        image.set_texture(cx, None);
        image.redraw(cx);
    }

    fn update_action_buttons(&mut self, cx: &mut Cx) {
        let viewer_active = self.mode == AppMode::Viewer;
        let host_active = self.mode == AppMode::Host;
        let show_cancel = viewer_active || (host_active && self.host_session_active);

        self.ui
            .widget(ids!(cancel_btn))
            .set_visible(cx, show_cancel);
        self.ui
            .widget(ids!(connect_btn))
            .set_visible(cx, !viewer_active);
        self.ui
            .widget(ids!(host_btn))
            .set_visible(cx, !viewer_active);

        let host_btn_text = if host_active {
            "Hosting"
        } else {
            "Return Host"
        };
        self.ui
            .widget(ids!(host_btn))
            .apply_over(cx, live! { text: (host_btn_text) });
    }

    fn cancel_current_connection(&mut self, cx: &mut Cx) {
        match self.mode {
            AppMode::Viewer => {
                self.stop_viewer_mode();
                self.mode = AppMode::Idle;
                self.update_mode_label(cx);
                self.set_remote(cx, "-");
                self.set_status(cx, "Connection cancelled");
                self.clear_video(cx);
            }
            AppMode::Host => {
                if let Some(handle) = self.host_handle.as_ref() {
                    handle.disconnect_session();
                }
                self.host_session_active = false;
                self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
                self.set_remote(cx, "-");
                self.set_status(cx, "Connection cancelled");
                self.clear_video(cx);
            }
            AppMode::Idle => {}
        }
        self.update_action_buttons(cx);
    }

    fn update_mode_label(&mut self, cx: &mut Cx) {
        let text = match self.mode {
            AppMode::Idle => "Mode: IDLE",
            AppMode::Host => "Mode: HOST",
            AppMode::Viewer => "Mode: VIEWER",
        };
        self.ui.label(ids!(mode_label)).set_text(cx, text);
    }

    fn set_side_panel_collapsed(&mut self, cx: &mut Cx, collapsed: bool) {
        self.panel_collapsed = collapsed;

        self.ui
            .view(ids!(panel_content))
            .set_visible(cx, !collapsed);
        self.ui
            .widget(ids!(panel_title))
            .set_visible(cx, !collapsed);
        self.ui
            .label(ids!(panel_title))
            .set_text(cx, if collapsed { "" } else { "Connection Panel" });

        let toggle_icon = if collapsed { ">" } else { "<" };
        self.ui
            .widget(ids!(panel_toggle_btn))
            .apply_over(cx, live! { text: (toggle_icon) });

        if collapsed {
            self.ui
                .widget(ids!(panel_title))
                .apply_over(cx, live! { width: 0.0 });

            self.ui.view(ids!(panel_header)).apply_over(
                cx,
                live! {
                    height: Fill,
                    align: { x: 0.5, y: 0.5 },
                    spacing: 0.0
                },
            );

            self.ui.view(ids!(side_panel)).apply_over(
                cx,
                live! {
                    width: 56.0,
                    padding: { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 }
                },
            );
        } else {
            self.ui
                .widget(ids!(panel_title))
                .apply_over(cx, live! { width: Fill });

            self.ui.view(ids!(panel_header)).apply_over(
                cx,
                live! {
                    height: Fit,
                    align: { x: 1.0, y: 0.5 },
                    spacing: 8.0
                },
            );

            self.ui.view(ids!(side_panel)).apply_over(
                cx,
                live! {
                    width: 360.0,
                    padding: { left: 12.0, right: 12.0, top: 12.0, bottom: 12.0 }
                },
            );
        }

        self.ui.view(ids!(side_panel)).redraw(cx);
    }

    fn stop_host_mode(&mut self) {
        if let Some(handle) = self.host_handle.take() {
            handle.stop();
        }
        self.host_session_active = false;
    }

    fn stop_viewer_mode(&mut self) {
        if let Some(handle) = self.viewer_handle.take() {
            handle.stop();
        }
        self.input_tx = None;
        self.input_capture_active = false;
        self.viewer_authorized = false;
    }

    fn start_host_mode(&mut self, cx: &mut Cx) {
        self.stop_viewer_mode();
        self.stop_host_mode();
        self.host_session_active = false;
        self.clear_video(cx);

        let bind_text = self.ui.text_input(ids!(host_bind_input)).text();
        self.host_bind = if bind_text.trim().is_empty() {
            DEFAULT_HOST_BIND.to_string()
        } else {
            bind_text.trim().to_string()
        };

        self.device_name = {
            let name = self.ui.text_input(ids!(device_name_input)).text();
            if name.trim().is_empty() {
                default_device_name()
            } else {
                name.trim().to_string()
            }
        };

        let bind_addr: SocketAddr = match self.host_bind.parse() {
            Ok(addr) => addr,
            Err(err) => {
                self.set_status(cx, &format!("Invalid host bind address: {err}"));
                self.mode = AppMode::Idle;
                self.update_mode_label(cx);
                self.update_action_buttons(cx);
                return;
            }
        };

        self.host_handle = Some(start_host_task(
            bind_addr,
            self.device_code.clone(),
            self.task_rx.sender(),
        ));
        self.mode = AppMode::Host;
        self.update_mode_label(cx);
        self.set_status(cx, "Starting host listener...");
        self.set_remote(cx, "-");
        self.update_action_buttons(cx);
    }

    fn start_viewer_mode(&mut self, cx: &mut Cx, host_addr: SocketAddr) {
        self.stop_host_mode();
        self.stop_viewer_mode();
        self.clear_video(cx);

        let viewer_code = self.ui.text_input(ids!(viewer_code_input)).text();
        let viewer_code = viewer_code.trim().to_string();
        if viewer_code.is_empty() {
            self.set_status(cx, "Target device code is empty");
            return;
        }

        self.device_name = {
            let name = self.ui.text_input(ids!(device_name_input)).text();
            if name.trim().is_empty() {
                default_device_name()
            } else {
                name.trim().to_string()
            }
        };

        let (input_tx, input_rx) = mpsc::unbounded_channel::<InputEvent>();
        self.input_tx = Some(input_tx);
        self.viewer_handle = Some(start_viewer_task(
            host_addr,
            self.device_name.clone(),
            viewer_code,
            self.task_rx.sender(),
            input_rx,
        ));
        self.mode = AppMode::Viewer;
        self.viewer_authorized = false;
        self.update_mode_label(cx);
        self.set_status(cx, &format!("Connecting viewer to {host_addr}..."));
        self.update_action_buttons(cx);
    }

    fn send_input_event(&self, event: InputEvent) {
        if self.mode != AppMode::Viewer || !self.viewer_authorized {
            return;
        }
        if let Some(tx) = &self.input_tx {
            let _ = tx.send(event);
        }
    }

    fn normalize_video_pos(&self, cx: &Cx, abs: Vec2d) -> Option<NormalizedPos> {
        let rect = self.ui.widget(ids!(video)).area().rect(cx);
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
            return None;
        }
        if !rect.contains(abs) {
            return None;
        }

        let x = ((abs.x - rect.pos.x) / rect.size.x).clamp(0.0, 1.0) as f32;
        let y = ((abs.y - rect.pos.y) / rect.size.y).clamp(0.0, 1.0) as f32;
        Some(NormalizedPos { x, y })
    }

    fn handle_remote_input(&mut self, cx: &Cx, event: &Event) {
        if self.mode != AppMode::Viewer || !self.viewer_authorized {
            return;
        }

        match event {
            Event::MouseMove(e) => {
                if let Some(pos) = self.normalize_video_pos(cx, e.abs) {
                    self.send_input_event(InputEvent::MouseMove { pos });
                }
            }
            Event::MouseDown(e) => {
                if let Some(pos) = self.normalize_video_pos(cx, e.abs) {
                    self.input_capture_active = true;
                    if let Some(button) = Self::map_mouse_button(e.button) {
                        self.send_input_event(InputEvent::MouseDown { pos, button });
                    }
                } else {
                    self.input_capture_active = false;
                }
            }
            Event::MouseUp(e) => {
                if let Some(pos) = self.normalize_video_pos(cx, e.abs) {
                    if let Some(button) = Self::map_mouse_button(e.button) {
                        self.send_input_event(InputEvent::MouseUp { pos, button });
                    }
                }
            }
            Event::Scroll(e) => {
                if let Some(pos) = self.normalize_video_pos(cx, e.abs) {
                    self.send_input_event(InputEvent::MouseScroll {
                        pos,
                        delta_x: e.scroll.x as f32,
                        delta_y: e.scroll.y as f32,
                    });
                }
            }
            Event::KeyDown(e) => {
                if self.input_capture_active {
                    self.send_input_event(InputEvent::KeyDown {
                        keycode: e.key_code as u32,
                        modifiers: Self::to_modifiers(&e.modifiers),
                    });
                }
            }
            Event::KeyUp(e) => {
                if self.input_capture_active {
                    self.send_input_event(InputEvent::KeyUp {
                        keycode: e.key_code as u32,
                        modifiers: Self::to_modifiers(&e.modifiers),
                    });
                }
            }
            _ => {}
        }
    }

    fn should_swallow_ui_keyboard_event(&self, event: &Event) -> bool {
        self.mode == AppMode::Viewer
            && self.viewer_authorized
            && self.input_capture_active
            && matches!(event, Event::KeyDown(_) | Event::KeyUp(_))
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        if !self.should_swallow_ui_keyboard_event(event) {
            self.ui.handle_event(cx, event, &mut Scope::empty());
        }
        self.match_event(cx, event);
        self.handle_remote_input(cx, event);
    }
}

fn is_peer_cancel_message(message: &str) -> bool {
    message.contains("peer canceled connection")
        || message.contains("session ended by host: Disconnected")
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "duplex-desk".to_string())
}

fn generate_device_code() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{:06}", nanos % 1_000_000)
}
