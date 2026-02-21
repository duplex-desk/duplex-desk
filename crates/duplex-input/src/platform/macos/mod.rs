use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::error::InputError;
use crate::event::{InputEvent, MouseButton, NormalizedPos};

pub struct InputInjector {
    display_width: f64,
    display_height: f64,
    event_source: CGEventSource,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

impl InputInjector {
    pub fn is_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn new() -> Result<Self, InputError> {
        let display = CGDisplay::main();
        let bounds = display.bounds();

        let event_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| InputError::InitFailed("CGEventSource".to_string()))?;

        Ok(Self {
            display_width: bounds.size.width,
            display_height: bounds.size.height,
            event_source,
        })
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

    fn normalized_to_point(&self, pos: &NormalizedPos) -> CGPoint {
        CGPoint::new(
            pos.x.clamp(0.0, 1.0) as f64 * self.display_width,
            pos.y.clamp(0.0, 1.0) as f64 * self.display_height,
        )
    }

    fn inject_mouse_move(&self, pos: &NormalizedPos) -> Result<(), InputError> {
        let point = self.normalized_to_point(pos);
        let event = CGEvent::new_mouse_event(
            self.event_source.clone(),
            CGEventType::MouseMoved,
            point,
            CGMouseButton::Left,
        )
        .map_err(|_| InputError::EventCreateFailed)?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn inject_mouse_button(
        &self,
        pos: &NormalizedPos,
        button: &MouseButton,
        pressed: bool,
    ) -> Result<(), InputError> {
        let point = self.normalized_to_point(pos);

        let (event_type, cg_button) = match (button, pressed) {
            (MouseButton::Left, true) => (CGEventType::LeftMouseDown, CGMouseButton::Left),
            (MouseButton::Left, false) => (CGEventType::LeftMouseUp, CGMouseButton::Left),
            (MouseButton::Right, true) => (CGEventType::RightMouseDown, CGMouseButton::Right),
            (MouseButton::Right, false) => (CGEventType::RightMouseUp, CGMouseButton::Right),
            (MouseButton::Middle, true) => (CGEventType::OtherMouseDown, CGMouseButton::Center),
            (MouseButton::Middle, false) => (CGEventType::OtherMouseUp, CGMouseButton::Center),
        };

        let event =
            CGEvent::new_mouse_event(self.event_source.clone(), event_type, point, cg_button)
                .map_err(|_| InputError::EventCreateFailed)?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn inject_scroll(
        &self,
        _pos: &NormalizedPos,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<(), InputError> {
        let event = CGEvent::new_scroll_event(
            self.event_source.clone(),
            ScrollEventUnit::PIXEL,
            2,
            delta_y as i32,
            delta_x as i32,
            0,
        )
        .map_err(|_| InputError::EventCreateFailed)?;

        event.post(CGEventTapLocation::HID);
        Ok(())
    }

    fn inject_key(
        &self,
        keycode: u32,
        modifiers: &crate::event::Modifiers,
        pressed: bool,
    ) -> Result<(), InputError> {
        let event = CGEvent::new_keyboard_event(self.event_source.clone(), keycode as u16, pressed)
            .map_err(|_| InputError::EventCreateFailed)?;

        let mut flags = CGEventFlags::empty();
        if modifiers.shift {
            flags |= CGEventFlags::CGEventFlagShift;
        }
        if modifiers.ctrl {
            flags |= CGEventFlags::CGEventFlagControl;
        }
        if modifiers.alt {
            flags |= CGEventFlags::CGEventFlagAlternate;
        }
        if modifiers.meta {
            flags |= CGEventFlags::CGEventFlagCommand;
        }
        event.set_flags(flags);

        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
