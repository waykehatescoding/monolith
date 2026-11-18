use iced::widget::{button, row, text};
use iced::{Alignment, Element, Theme};
use monolith_theme::token::{INK_TERTIARY, SPACE_2, TEXT_MD};
use monolith_theme::{icon_colored, Icon, UI_MEDIUM};

pub fn busy_button<'a, M: Clone + 'a>(
    label: &'a str,
    busy_label: &'a str,
    busy: bool,
    on_press: Option<M>,
    look: fn(&Theme, button::Status) -> button::Style,
) -> Element<'a, M> {
    let content: Element<'a, M> = if busy {
        row![
            icon_colored(Icon::Restart, 16.0, INK_TERTIARY),
            text(busy_label).size(TEXT_MD).font(UI_MEDIUM)
        ]
        .spacing(SPACE_2)
        .align_y(Alignment::Center)
        .into()
    } else {
        text(label).size(TEXT_MD).font(UI_MEDIUM).into()
    };
    let base = button(content).padding([8, 16]).style(move |theme, status| {
        look(
            theme,
            if busy {
                button::Status::Disabled
            } else {
                status
            },
        )
    });
    match on_press {
        Some(message) if !busy => base.on_press(message).into(),
        _ => base.into(),
    }
}
