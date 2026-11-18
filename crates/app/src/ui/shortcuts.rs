use iced::keyboard::key::{Code, Named, Physical};
use iced::keyboard::{Event as KeyEvent, Key};
use iced::{event, window, Event};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Palette,
    NewServer,
    Settings,
    Server(usize),
    Escape,
}

pub const BINDINGS: [(&str, &str); 5] = [
    ("Ctrl+K", "Command palette"),
    ("Ctrl+N", "New server"),
    ("Ctrl+,", "Settings"),
    ("Ctrl+1 to 9", "Open server"),
    ("Esc", "Close dialog or panel"),
];

pub fn from_event(event: Event, status: event::Status, _: window::Id) -> Option<Shortcut> {
    let Event::Keyboard(KeyEvent::KeyPressed {
        key,
        physical_key,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    if matches!(key, Key::Named(Named::Escape)) {
        return (status == event::Status::Ignored).then_some(Shortcut::Escape);
    }
    if !modifiers.command() || modifiers.alt() {
        return None;
    }
    let Physical::Code(code) = physical_key else {
        return None;
    };
    match code {
        Code::KeyK => Some(Shortcut::Palette),
        Code::KeyN => Some(Shortcut::NewServer),
        Code::Comma => Some(Shortcut::Settings),
        Code::Digit1 | Code::Numpad1 => Some(Shortcut::Server(0)),
        Code::Digit2 | Code::Numpad2 => Some(Shortcut::Server(1)),
        Code::Digit3 | Code::Numpad3 => Some(Shortcut::Server(2)),
        Code::Digit4 | Code::Numpad4 => Some(Shortcut::Server(3)),
        Code::Digit5 | Code::Numpad5 => Some(Shortcut::Server(4)),
        Code::Digit6 | Code::Numpad6 => Some(Shortcut::Server(5)),
        Code::Digit7 | Code::Numpad7 => Some(Shortcut::Server(6)),
        Code::Digit8 | Code::Numpad8 => Some(Shortcut::Server(7)),
        Code::Digit9 | Code::Numpad9 => Some(Shortcut::Server(8)),
        _ => None,
    }
}
