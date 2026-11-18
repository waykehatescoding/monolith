mod font;
mod icon;
pub mod style;
pub mod token;
pub mod widget;

pub use font::{FONT_BYTES, MONO, MONO_BOLD, UI, UI_BOLD, UI_MEDIUM, UI_SEMIBOLD};
pub use icon::{app_icon, app_mark, icon, icon_colored, Icon};

use iced::theme::Palette;
use iced::Theme;

pub fn theme() -> Theme {
    Theme::custom(
        "Monolith",
        Palette {
            background: token::GROUND,
            text: token::INK,
            primary: token::MOSS,
            success: token::MOSS,
            warning: token::AMBER,
            danger: token::CORAL,
        },
    )
}
