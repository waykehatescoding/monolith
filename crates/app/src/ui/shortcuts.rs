use iced::keyboard::key::Named;
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
    ("Esc", "Close dialog"),
];

pub fn from_event(event: Event, _: event::Status, _: window::Id) -> Option<Shortcut> {
    let Event::Keyboard(KeyEvent::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    if matches!(key, Key::Named(Named::Escape)) {
        return Some(Shortcut::Escape);
    }
    if !modifiers.command() || modifiers.alt() {
        return None;
    }
    let Key::Character(character) = key else {
        return None;
    };
    match character.as_str() {
        "k" => Some(Shortcut::Palette),
        "n" => Some(Shortcut::NewServer),
        "," => Some(Shortcut::Settings),
        digit => digit
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=9).contains(n))
            .map(|n| Shortcut::Server(n - 1)),
    }
}
