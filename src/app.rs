use std::net::SocketAddr;

use duple_x_scap::frame::DuplexScapFrame;
use makepad_components::makepad_widgets::*;

use crate::{receiver_task::start_receiver_task, video_view::VideoFrameTexture};

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    use makepad_components::button::*;
    use makepad_components::input::*;
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

                        host_hint = <MpInput> {
                            width: 260,
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
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_components::makepad_widgets::live_design(cx);
        makepad_components::live_design(cx);
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        if self.receiver_started {
            return;
        }

        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();

        let host_arg = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "127.0.0.1:5000".to_string());

        if let Some(mut input) = self.ui.text_input(ids!(host_hint)).borrow_mut() {
            input.set_text(cx, &host_arg);
        }

        let host_addr: SocketAddr = match host_arg.parse() {
            Ok(addr) => addr,
            Err(err) => {
                tracing::error!("invalid host address '{host_arg}': {err}");
                return;
            }
        };

        tracing::info!("starting receiver task: {host_addr}");
        start_receiver_task(host_addr, self.frame_rx.sender());
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

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
