use std::net::SocketAddr;

use duple_x_input::{InputEvent, Modifiers, MouseButton as InputMouseButton, NormalizedPos};
use duple_x_scap::frame::DuplexScapFrame;
use makepad_components::makepad_widgets::*;
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{receiver_task::start_receiver_task, video_view::VideoFrameTexture};

live_design! {
    use link::theme::*;
    use link::theme_colors::*;
    use link::shaders::*;
    use link::widgets::*;

    use makepad_components::button::*;
    use makepad_components::tooltip::*;
    use makepad_components::modal::*;

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { title: "Duplex Desk Viewer" }

                body = <View> {
                    width: Fill,
                    height: Fill,
                    flow: Overlay,

                    video = <Image> {
                        width: Fill,
                        height: Fill,
                        fit: Stretch,
                    }

                    overlay = <View> {
                        width: Fill,
                        height: Fit,
                        flow: Right,
                        spacing: 10,
                        align: { y: 0.5 },
                        padding: { left: 12, right: 12, top: 12, bottom: 0 },

                        host_hint = <Label> {
                            width: Fit,
                            draw_text: {
                                color: (MUTED_FOREGROUND),
                            }
                            text: "127.0.0.1:5000",
                        }

                        status = <MpButtonSecondary> {
                            text: "Viewer Running",
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust(ToUIReceiver::default())]
    frame_rx: ToUIReceiver<DuplexScapFrame>,
    #[rust(VideoFrameTexture::default())]
    video: VideoFrameTexture,
    #[rust(false)]
    receiver_started: bool,
    #[rust(None)]
    input_tx: Option<UnboundedSender<InputEvent>>,
    #[rust(false)]
    input_capture_active: bool,
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
    fn handle_startup(&mut self, _cx: &mut Cx) {
        if self.receiver_started {
            return;
        }

        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        let host_arg = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "127.0.0.1:5000".to_string());

        let host_addr: SocketAddr = match host_arg.parse() {
            Ok(addr) => addr,
            Err(err) => {
                tracing::error!("invalid host address '{host_arg}': {err}");
                return;
            }
        };

        let (input_tx, input_rx) = mpsc::unbounded_channel::<InputEvent>();
        self.input_tx = Some(input_tx);

        tracing::info!("starting receiver task: {host_addr}");
        start_receiver_task(host_addr, self.frame_rx.sender(), input_rx);
        self.receiver_started = true;
    }

    fn handle_signal(&mut self, cx: &mut Cx) {
        let mut latest: Option<DuplexScapFrame> = None;
        while let Ok(frame) = self.frame_rx.try_recv() {
            latest = Some(frame);
        }

        let Some(frame) = latest else {
            return;
        };

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

    fn handle_actions(&mut self, _cx: &mut Cx, _actions: &Actions) {}
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

    fn send_input_event(&self, event: InputEvent) {
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
        self.match_event(cx, event);
        self.handle_remote_input(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
