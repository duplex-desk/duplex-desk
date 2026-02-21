use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    VIRTUAL_KEY, VK_ADD, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE,
    VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9,
    VK_F10, VK_F11, VK_F12, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_MULTIPLY, VK_NEXT,
    VK_NUMLOCK, VK_NUMPAD0, VK_NUMPAD1, VK_NUMPAD2, VK_NUMPAD3, VK_NUMPAD4, VK_NUMPAD5, VK_NUMPAD6,
    VK_NUMPAD7, VK_NUMPAD8, VK_NUMPAD9, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6,
    VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PAUSE, VK_PRINT, VK_PRIOR,
    VK_RETURN, VK_RIGHT, VK_SCROLL, VK_SHIFT, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};

use crate::error::InputError;
use crate::event::{InputEvent, Modifiers, MouseButton, NormalizedPos};

pub struct InputInjector;

impl InputInjector {
    pub fn is_trusted() -> bool {
        true
    }

    pub fn new() -> Result<Self, InputError> {
        Ok(Self)
    }

    pub fn inject(&self, event: &InputEvent) -> Result<(), InputError> {
        match event {
            InputEvent::MouseMove { pos } => self.inject_mouse_move(pos),
            InputEvent::MouseDown { pos, button } => self.inject_mouse_button(pos, button, true),
            InputEvent::MouseUp { pos, button } => self.inject_mouse_button(pos, button, false),
            InputEvent::MouseScroll {
                pos,
                delta_x,
                delta_y,
            } => self.inject_scroll(pos, *delta_x, *delta_y),
            InputEvent::KeyDown { keycode, modifiers } => {
                self.inject_key(*keycode, modifiers, true)
            }
            InputEvent::KeyUp { keycode, modifiers } => self.inject_key(*keycode, modifiers, false),
        }
    }

    fn inject_mouse_move(&self, pos: &NormalizedPos) -> Result<(), InputError> {
        let (dx, dy) = normalized_to_absolute(pos);
        send_inputs(&[mouse_input(
            dx,
            dy,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        )])
    }

    fn inject_mouse_button(
        &self,
        pos: &NormalizedPos,
        button: &MouseButton,
        pressed: bool,
    ) -> Result<(), InputError> {
        let (dx, dy) = normalized_to_absolute(pos);
        let click_flag = match (button, pressed) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };

        send_inputs(&[mouse_input(
            dx,
            dy,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | click_flag,
        )])
    }

    fn inject_scroll(
        &self,
        pos: &NormalizedPos,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<(), InputError> {
        let (dx, dy) = normalized_to_absolute(pos);
        let mut inputs = Vec::with_capacity(3);
        inputs.push(mouse_input(
            dx,
            dy,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        ));

        let wheel_delta_y = (delta_y * 120.0).round() as i32;
        if wheel_delta_y != 0 {
            inputs.push(mouse_input(0, 0, wheel_delta_y as u32, MOUSEEVENTF_WHEEL));
        }

        let wheel_delta_x = (delta_x * 120.0).round() as i32;
        if wheel_delta_x != 0 {
            inputs.push(mouse_input(0, 0, wheel_delta_x as u32, MOUSEEVENTF_HWHEEL));
        }

        if inputs.len() == 1 {
            return Ok(());
        }
        send_inputs(&inputs)
    }

    fn inject_key(
        &self,
        keycode: u32,
        modifiers: &Modifiers,
        pressed: bool,
    ) -> Result<(), InputError> {
        let Some(vk) = map_keycode_to_vk(keycode) else {
            return Err(InputError::InjectFailed(format!(
                "unsupported keycode: {keycode}"
            )));
        };

        let mut inputs = Vec::with_capacity(8);
        if pressed {
            push_modifier_down(&mut inputs, modifiers);
            inputs.push(keyboard_input(vk, KEYBD_EVENT_FLAGS(0)));
        } else {
            inputs.push(keyboard_input(vk, KEYEVENTF_KEYUP));
            push_modifier_up(&mut inputs, modifiers);
        }
        send_inputs(&inputs)
    }
}

fn normalized_to_absolute(pos: &NormalizedPos) -> (i32, i32) {
    let x = (pos.x.clamp(0.0, 1.0) * 65535.0).round() as i32;
    let y = (pos.y.clamp(0.0, 1.0) * 65535.0).round() as i32;
    (x, y)
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), InputError> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(InputError::InjectFailed(format!(
            "SendInput injected {sent}/{} events",
            inputs.len()
        )))
    }
}

fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn keyboard_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn push_modifier_down(inputs: &mut Vec<INPUT>, mods: &Modifiers) {
    if mods.ctrl {
        inputs.push(keyboard_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)));
    }
    if mods.alt {
        inputs.push(keyboard_input(VK_MENU, KEYBD_EVENT_FLAGS(0)));
    }
    if mods.shift {
        inputs.push(keyboard_input(VK_SHIFT, KEYBD_EVENT_FLAGS(0)));
    }
    if mods.meta {
        inputs.push(keyboard_input(VK_LWIN, KEYBD_EVENT_FLAGS(0)));
    }
}

fn push_modifier_up(inputs: &mut Vec<INPUT>, mods: &Modifiers) {
    if mods.meta {
        inputs.push(keyboard_input(VK_LWIN, KEYEVENTF_KEYUP));
    }
    if mods.shift {
        inputs.push(keyboard_input(VK_SHIFT, KEYEVENTF_KEYUP));
    }
    if mods.alt {
        inputs.push(keyboard_input(VK_MENU, KEYEVENTF_KEYUP));
    }
    if mods.ctrl {
        inputs.push(keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP));
    }
}

fn map_keycode_to_vk(keycode: u32) -> Option<VIRTUAL_KEY> {
    // makepad KeyCode enum ordinals (Escape = 0 ... ArrowRight = 99).
    let vk = match keycode {
        0 => VK_ESCAPE,
        1 => VK_BACK,
        2 => VK_OEM_3,
        3 => VIRTUAL_KEY(b'0' as u16),
        4 => VIRTUAL_KEY(b'1' as u16),
        5 => VIRTUAL_KEY(b'2' as u16),
        6 => VIRTUAL_KEY(b'3' as u16),
        7 => VIRTUAL_KEY(b'4' as u16),
        8 => VIRTUAL_KEY(b'5' as u16),
        9 => VIRTUAL_KEY(b'6' as u16),
        10 => VIRTUAL_KEY(b'7' as u16),
        11 => VIRTUAL_KEY(b'8' as u16),
        12 => VIRTUAL_KEY(b'9' as u16),
        13 => VK_OEM_MINUS,
        14 => VK_OEM_PLUS,
        15 => VK_BACK,
        16 => VK_TAB,
        17 => VIRTUAL_KEY(b'Q' as u16),
        18 => VIRTUAL_KEY(b'W' as u16),
        19 => VIRTUAL_KEY(b'E' as u16),
        20 => VIRTUAL_KEY(b'R' as u16),
        21 => VIRTUAL_KEY(b'T' as u16),
        22 => VIRTUAL_KEY(b'Y' as u16),
        23 => VIRTUAL_KEY(b'U' as u16),
        24 => VIRTUAL_KEY(b'I' as u16),
        25 => VIRTUAL_KEY(b'O' as u16),
        26 => VIRTUAL_KEY(b'P' as u16),
        27 => VK_OEM_4,
        28 => VK_OEM_6,
        29 => VK_RETURN,
        30 => VIRTUAL_KEY(b'A' as u16),
        31 => VIRTUAL_KEY(b'S' as u16),
        32 => VIRTUAL_KEY(b'D' as u16),
        33 => VIRTUAL_KEY(b'F' as u16),
        34 => VIRTUAL_KEY(b'G' as u16),
        35 => VIRTUAL_KEY(b'H' as u16),
        36 => VIRTUAL_KEY(b'J' as u16),
        37 => VIRTUAL_KEY(b'K' as u16),
        38 => VIRTUAL_KEY(b'L' as u16),
        39 => VK_OEM_1,
        40 => VK_OEM_7,
        41 => VK_OEM_5,
        42 => VIRTUAL_KEY(b'Z' as u16),
        43 => VIRTUAL_KEY(b'X' as u16),
        44 => VIRTUAL_KEY(b'C' as u16),
        45 => VIRTUAL_KEY(b'V' as u16),
        46 => VIRTUAL_KEY(b'B' as u16),
        47 => VIRTUAL_KEY(b'N' as u16),
        48 => VIRTUAL_KEY(b'M' as u16),
        49 => VK_OEM_COMMA,
        50 => VK_OEM_PERIOD,
        51 => VK_OEM_2,
        52 => VK_CONTROL,
        53 => VK_MENU,
        54 => VK_SHIFT,
        55 => VK_LWIN,
        56 => VK_SPACE,
        57 => VK_CAPITAL,
        58 => VK_F1,
        59 => VK_F2,
        60 => VK_F3,
        61 => VK_F4,
        62 => VK_F5,
        63 => VK_F6,
        64 => VK_F7,
        65 => VK_F8,
        66 => VK_F9,
        67 => VK_F10,
        68 => VK_F11,
        69 => VK_F12,
        70 => VK_PRINT,
        71 => VK_SCROLL,
        72 => VK_PAUSE,
        73 => VK_INSERT,
        74 => VK_DELETE,
        75 => VK_HOME,
        76 => VK_END,
        77 => VK_PRIOR,
        78 => VK_NEXT,
        79 => VK_NUMPAD0,
        80 => VK_NUMPAD1,
        81 => VK_NUMPAD2,
        82 => VK_NUMPAD3,
        83 => VK_NUMPAD4,
        84 => VK_NUMPAD5,
        85 => VK_NUMPAD6,
        86 => VK_NUMPAD7,
        87 => VK_NUMPAD8,
        88 => VK_NUMPAD9,
        89 => VK_OEM_PLUS,
        90 => VK_SUBTRACT,
        91 => VK_ADD,
        92 => VK_DECIMAL,
        93 => VK_MULTIPLY,
        94 => VK_DIVIDE,
        95 => VK_NUMLOCK,
        96 => VK_RETURN,
        97 => VK_UP,
        98 => VK_DOWN,
        99 => VK_LEFT,
        100 => VK_RIGHT,
        _ if keycode > 101 && keycode <= 0xFF => VIRTUAL_KEY(keycode as u16),
        _ => return None,
    };
    Some(vk)
}
