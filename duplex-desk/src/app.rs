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
    task_event::TaskEvent,
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

#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust(ToUIReceiver::default())]
    task_rx: ToUIReceiver<TaskEvent>,
    #[rust(VideoFrameTexture::default())]
    video: VideoFrameTexture,
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
        self.update_mode_label(cx);
        self.set_remote(cx, "-");
        self.set_status(
            cx,
            "Idle. Connect as viewer or click Return Host to accept requests.",
        );
        self.set_side_panel_collapsed(cx, false);
    }

    fn handle_signal(&mut self, cx: &mut Cx) {
        let mut latest_frame = None;

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
                    self.ui
                        .label(ids!(auth_remote_label))
                        .set_text(cx, &format!("Remote: {remote_addr}"));
                    self.ui
                        .label(ids!(auth_device_label))
                        .set_text(cx, &format!("Device: {device_name}"));
                    self.ui.mp_modal_widget(ids!(auth_modal)).open(cx);
                    self.set_remote(cx, &remote_addr.to_string());
                    self.set_status(cx, "Incoming control request");
                }
                TaskEvent::HostStopped(message) => {
                    if self.mode == AppMode::Host {
                        self.host_handle = None;
                        self.mode = AppMode::Idle;
                        self.update_mode_label(cx);
                        self.set_status(cx, &message);
                    }
                }
                TaskEvent::ViewerConnected(addr) => {
                    if self.mode == AppMode::Viewer {
                        self.set_remote(cx, &addr.to_string());
                        self.set_status(cx, &format!("Connected, waiting auth: {addr}"));
                    }
                }
                TaskEvent::ViewerAuthResult { accepted, reason } => {
                    if self.mode == AppMode::Viewer {
                        self.viewer_authorized = accepted;
                        if accepted {
                            self.set_status(cx, &format!("Viewer authorized: {reason}"));
                        } else {
                            self.set_status(cx, &format!("Viewer rejected: {reason}"));
                        }
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
                    }
                }
            }
        }

        if let Some(frame) = latest_frame {
            if self.video.update_frame(
                cx,
                &frame.data,
                frame.width as usize,
                frame.height as usize,
                frame.stride as usize,
            ) {
                let image = self.ui.image(ids!(video));
                image.set_texture(cx, self.video.texture());
                image.redraw(cx);
            }
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        if self.ui.mp_button(ids!(connect_btn)).clicked(actions) {
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

        if self.ui.mp_button(ids!(host_btn)).clicked(actions) {
            self.start_host_mode(cx);
        }

        if self.ui.mp_button(ids!(auth_allow_btn)).clicked(actions) {
            if let Some(handle) = self.host_handle.as_ref() {
                handle.approve();
            }
            self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
            self.set_status(cx, "Auth approved");
        }

        if self.ui.mp_button(ids!(auth_reject_btn)).clicked(actions) {
            if let Some(handle) = self.host_handle.as_ref() {
                handle.reject();
            }
            self.ui.mp_modal_widget(ids!(auth_modal)).close(cx);
            self.set_status(cx, "Auth rejected");
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
    }

    fn start_viewer_mode(&mut self, cx: &mut Cx, host_addr: SocketAddr) {
        self.stop_host_mode();
        self.stop_viewer_mode();

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
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.ui.handle_event(cx, event, &mut Scope::empty());
        self.match_event(cx, event);
        self.handle_remote_input(cx, event);
    }
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
