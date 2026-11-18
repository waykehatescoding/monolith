use iced::widget::{button, column, container, row, text, Space};
use monolith_theme::token::{BUTTON_MD};
use iced::{Element, Length, Theme};
use monolith_theme::token::{SPACE_2, SPACE_5, TEXT_LG, TEXT_MD};
use monolith_theme::{style, UI_SEMIBOLD};

use super::busy::busy_button;
use super::overlay::modal;

pub fn dialog<'a, M: Clone + 'a>(
    title: &'a str,
    body: &'a str,
    cancel_label: &'a str,
    confirm_label: &'a str,
    busy_label: &'a str,
    busy: bool,
    danger: bool,
    on_cancel: M,
    on_confirm: M,
) -> Element<'a, M> {
    let confirm_style: fn(&Theme, button::Status) -> button::Style = if danger {
        style::danger
    } else {
        style::primary
    };
    let card = container(
        column![
            column![
                text(title).size(TEXT_LG).font(UI_SEMIBOLD),
                text(body).size(TEXT_MD).style(style::muted)
            ]
            .spacing(SPACE_2),
            row![
                Space::new().width(Length::Fill),
                busy_button(cancel_label, cancel_label, false, (!busy).then(|| on_cancel.clone()), style::ghost, BUTTON_MD),
                busy_button(confirm_label, busy_label, busy, Some(on_confirm), confirm_style, BUTTON_MD),
            ]
            .spacing(SPACE_2)
        ]
        .spacing(SPACE_5),
    )
    .padding(SPACE_5)
    .width(420)
    .style(style::overlay);
    modal(card, on_cancel, None)
}
